import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { Pin, Trash2, Archive, Edit3, Loader2, MoreHorizontal, SlidersHorizontal } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { chatApi, ChatSession } from '../../api/chat';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { SearchInput } from '../ui/search-input';
import { formatRelativeTime } from '../../utils/format';

interface ChatSessionListProps {
  teamId: string;
  agentId?: string;
  selectedSessionId: string | null;
  onSelectSession: (session: ChatSession) => void;
  onSessionRemoved?: (sessionId: string) => void;
}

const PAGE_SIZE = 20;

function mergeSessionsById(current: ChatSession[], incoming: ChatSession[]): ChatSession[] {
  const merged = [...current];
  const seen = new Set(current.map((session) => session.session_id));
  for (const session of incoming) {
    if (seen.has(session.session_id)) {
      const index = merged.findIndex((item) => item.session_id === session.session_id);
      if (index >= 0) {
        merged[index] = session;
      }
      continue;
    }
    seen.add(session.session_id);
    merged.push(session);
  }
  return merged;
}

/** Strip markdown syntax and collapse whitespace for clean preview */
function sanitizePreview(text?: string): string {
  if (!text) return '';
  return text
    .replace(/```[\s\S]*?```/g, ' ')   // code blocks
    .replace(/`[^`]+`/g, ' ')          // inline code
    .replace(/#{1,6}\s*/g, '')          // headings
    .replace(/[*_~]{1,3}/g, '')         // bold/italic/strikethrough
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1') // links/images
    .replace(/[-=]{3,}/g, ' ')          // hr
    .replace(/<[^>]+>/g, '')            // html tags
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 50);
}

/** Detect UUID-like strings */
function isUUID(s: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s);
}

function groupByDate(sessions: ChatSession[]) {
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const yesterday = new Date(today.getTime() - 86400000);
  const week = new Date(today.getTime() - 7 * 86400000);

  const groups: { label: string; items: ChatSession[] }[] = [
    { label: 'pinned', items: [] },
    { label: 'today', items: [] },
    { label: 'yesterday', items: [] },
    { label: 'previous7Days', items: [] },
    { label: 'older', items: [] },
  ];

  for (const s of sessions) {
    if (s.pinned) {
      groups[0].items.push(s);
      continue;
    }
    const d = s.last_message_at ? new Date(s.last_message_at) : new Date(s.created_at);
    if (d >= today) groups[1].items.push(s);
    else if (d >= yesterday) groups[2].items.push(s);
    else if (d >= week) groups[3].items.push(s);
    else groups[4].items.push(s);
  }

  return groups.filter(g => g.items.length > 0);
}

export function ChatSessionList({
  teamId,
  agentId,
  selectedSessionId,
  onSelectSession,
  onSessionRemoved,
}: ChatSessionListProps) {
  const { t } = useTranslation();
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [loadingList, setLoadingList] = useState(true);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [page, setPage] = useState(1);
  const [hasMore, setHasMore] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [menuSessionId, setMenuSessionId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState('');
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [timeFilter, setTimeFilter] = useState<'all' | 'today' | 'week' | 'month' | 'older'>('all');
  const [includeHidden, setIncludeHidden] = useState(false);
  const [filterOpen, setFilterOpen] = useState(false);
  const [draftTimeFilter, setDraftTimeFilter] = useState<'all' | 'today' | 'week' | 'month' | 'older'>('all');
  const [draftIncludeHidden, setDraftIncludeHidden] = useState(false);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const shouldAutoLoadMore = hasMore && !loadingList && !isLoadingMore && !filterOpen && !menuSessionId;

  const fetchSessionsPage = useCallback(
    async (targetPage: number) => {
      return chatApi.listSessions(teamId, agentId, targetPage, PAGE_SIZE, undefined, includeHidden);
    },
    [teamId, agentId, includeHidden],
  );

  const loadSessions = useCallback(async () => {
    setLoadingList(true);
    setLoadError(false);
    try {
      const list = await fetchSessionsPage(1);
      setSessions(list);
      setPage(1);
      setHasMore(list.length === PAGE_SIZE);
    } catch (e) {
      console.error('Failed to load sessions:', e);
      setLoadError(true);
      setSessions([]);
      setHasMore(false);
    } finally {
      setLoadingList(false);
    }
  }, [fetchSessionsPage]);

  const loadMoreSessions = useCallback(async () => {
    if (loadingList || isLoadingMore || !hasMore) {
      return;
    }
    setIsLoadingMore(true);
    setLoadError(false);
    const nextPage = page + 1;
    try {
      const list = await fetchSessionsPage(nextPage);
      setSessions((current) => mergeSessionsById(current, list));
      setPage(nextPage);
      setHasMore(list.length === PAGE_SIZE);
    } catch (e) {
      console.error('Failed to load more sessions:', e);
      setLoadError(true);
    } finally {
      setIsLoadingMore(false);
    }
  }, [fetchSessionsPage, hasMore, isLoadingMore, loadingList, page]);

  useEffect(() => {
    loadSessions();
  }, [loadSessions]);

  useEffect(() => {
    if (!scrollContainerRef.current || !sentinelRef.current) {
      return;
    }
    if (!shouldAutoLoadMore) {
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        const [entry] = entries;
        if (entry?.isIntersecting) {
          void loadMoreSessions();
        }
      },
      {
        root: scrollContainerRef.current,
        rootMargin: '0px 0px 120px 0px',
        threshold: 0.1,
      },
    );

    observer.observe(sentinelRef.current);
    return () => observer.disconnect();
  }, [loadMoreSessions, shouldAutoLoadMore]);

  const handleScroll = useCallback(() => {
    const root = scrollContainerRef.current;
    if (!root || !shouldAutoLoadMore) {
      return;
    }

    const remaining = root.scrollHeight - root.scrollTop - root.clientHeight;
    if (remaining <= 160) {
      void loadMoreSessions();
    }
  }, [loadMoreSessions, shouldAutoLoadMore]);

  useEffect(() => {
    const root = scrollContainerRef.current;
    if (!root || !shouldAutoLoadMore) {
      return;
    }

    if (root.scrollHeight <= root.clientHeight + 24) {
      void loadMoreSessions();
    }
  }, [sessions.length, loadMoreSessions, shouldAutoLoadMore]);

  const closeMenu = () => {
    setMenuSessionId(null);
    setDraftTimeFilter(timeFilter);
    setDraftIncludeHidden(includeHidden);
    setFilterOpen(false);
  };

  const openFilterPanel = () => {
    setDraftTimeFilter(timeFilter);
    setDraftIncludeHidden(includeHidden);
    setFilterOpen(true);
  };

  const cancelFilters = () => {
    setDraftTimeFilter(timeFilter);
    setDraftIncludeHidden(includeHidden);
    setFilterOpen(false);
  };

  const applyFilters = () => {
    setTimeFilter(draftTimeFilter);
    setIncludeHidden(draftIncludeHidden);
    setFilterOpen(false);
  };

  const handleRename = (sessionId: string) => {
    closeMenu();
    const session = sessions.find(s => s.session_id === sessionId);
    setEditingId(sessionId);
    setEditTitle(session?.title || '');
  };

  const submitRename = async () => {
    if (!editingId || !editTitle.trim()) {
      setEditingId(null);
      return;
    }
    try {
      await chatApi.renameSession(editingId, editTitle.trim());
      await loadSessions();
    } catch (e) {
      console.error('Failed to rename:', e);
    }
    setEditingId(null);
  };

  const cancelRename = () => setEditingId(null);

  const handlePin = async (sessionId: string) => {
    closeMenu();
    const session = sessions.find(s => s.session_id === sessionId);
    if (!session) return;
    try {
      await chatApi.pinSession(sessionId, !session.pinned);
      await loadSessions();
    } catch (e) {
      console.error('Failed to pin:', e);
    }
  };

  const handleArchive = async (sessionId: string) => {
    closeMenu();
    try {
      await chatApi.archiveSession(sessionId);
      await loadSessions();
      if (sessionId === selectedSessionId) {
        onSessionRemoved?.(sessionId);
      }
    } catch (e) {
      console.error('Failed to archive:', e);
    }
  };

  const handleDelete = async (sessionId: string) => {
    closeMenu();
    setDeleteTarget(sessionId);
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    try {
      await chatApi.deleteSession(deleteTarget);
      await loadSessions();
      if (deleteTarget === selectedSessionId) {
        onSessionRemoved?.(deleteTarget);
      }
    } catch (e) {
      console.error('Failed to delete:', e);
    } finally {
      setDeleteTarget(null);
    }
  };

  const filteredSessions = useMemo(() => {
    let result = sessions;

    // Text search
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase();
      result = result.filter(s =>
        (s.title || '').toLowerCase().includes(q) ||
        (s.last_message_preview || '').toLowerCase().includes(q)
      );
    }

    // Time filter
    if (timeFilter !== 'all') {
      const now = new Date();
      const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
      const weekAgo = new Date(today.getTime() - 7 * 86400000);
      const monthAgo = new Date(today.getTime() - 30 * 86400000);

      result = result.filter(s => {
        const d = s.last_message_at ? new Date(s.last_message_at) : new Date(s.created_at);
        switch (timeFilter) {
          case 'today': return d >= today;
          case 'week': return d >= weekAgo;
          case 'month': return d >= monthAgo;
          case 'older': return d < monthAgo;
          default: return true;
        }
      });
    }

    return result;
  }, [sessions, searchQuery, timeFilter]);

  const groups = groupByDate(filteredSessions);
  const groupLabels: Record<string, string> = {
    pinned: t('chat.pinned', 'Pinned'),
    today: t('chat.today', 'Today'),
    yesterday: t('chat.yesterday', 'Yesterday'),
    previous7Days: t('chat.previous7Days', 'Previous 7 Days'),
    older: t('chat.older', 'Older'),
  };

  return (
    <div className="flex flex-col h-full min-h-0" onClick={closeMenu}>
      {/* Search */}
      <div className="px-2 py-1.5 border-b">
        <div className="relative">
          <div className="flex items-center gap-1.5">
            <div className="flex-1 min-w-0">
              <SearchInput
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                onClear={() => setSearchQuery('')}
                placeholder={t('chat.searchSessions', 'Search sessions...')}
                className="h-8 text-xs"
              />
            </div>
            <button
              type="button"
              className={`h-8 w-8 shrink-0 inline-flex items-center justify-center rounded-md border transition-colors ${
                filterOpen ? 'bg-accent border-border' : 'bg-background hover:bg-accent/50 border-border/70'
              }`}
              onClick={(e) => {
                e.stopPropagation();
                if (filterOpen) {
                  cancelFilters();
                } else {
                  openFilterPanel();
                }
              }}
              title={t('chat.filters', '筛选')}
              aria-label={t('chat.filters', '筛选')}
            >
              <SlidersHorizontal className="h-3.5 w-3.5 text-muted-foreground" />
            </button>
          </div>

          {filterOpen && (
            <div
              className="absolute right-0 top-9 z-30 w-56 rounded-md border bg-popover shadow-md p-2"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="text-caption font-medium text-foreground/80 mb-1">
                {t('chat.filters', '筛选')}
              </div>
              <label className="inline-flex items-center gap-2 text-caption text-muted-foreground cursor-pointer select-none">
                <input
                  type="checkbox"
                  className="h-3.5 w-3.5 rounded border-border"
                  checked={draftIncludeHidden}
                  onChange={(e) => setDraftIncludeHidden(e.target.checked)}
                />
                {t('chat.showHiddenSessions', '显示隐藏会话（任务/系统）')}
              </label>

              <div className="mt-2 grid grid-cols-3 gap-1">
                {(['all', 'today', 'week', 'month', 'older'] as const).map(f => (
                  <button
                    key={f}
                    onClick={() => setDraftTimeFilter(f)}
                    className={`px-2 py-1 rounded-md text-caption transition-colors ${
                      draftTimeFilter === f
                        ? 'bg-primary text-primary-foreground'
                        : 'bg-muted text-muted-foreground hover:bg-muted/80'
                    }`}
                  >
                    {t(`chat.filter${f.charAt(0).toUpperCase() + f.slice(1)}`)}
                  </button>
                ))}
              </div>

              <div className="mt-2 pt-2 border-t border-border/60 flex items-center justify-end gap-1.5">
                <button
                  type="button"
                  className="px-2 py-1 rounded-md text-caption border border-border/70 text-muted-foreground hover:bg-muted/60"
                  onClick={cancelFilters}
                >
                  {t('common.cancel', '取消')}
                </button>
                <button
                  type="button"
                  className="px-2 py-1 rounded-md text-caption bg-primary text-primary-foreground hover:opacity-90"
                  onClick={applyFilters}
                >
                  {t('common.confirm', '确认')}
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
      {/* Session list */}
      <div
        ref={scrollContainerRef}
        className="min-h-0 flex-1 overflow-y-auto scrollbar-subtle"
        onScroll={handleScroll}
      >
        {loadingList && sessions.length === 0 && (
          <div className="flex items-center justify-center p-4">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        )}
        {!loadingList && sessions.length === 0 && (
          <div className="space-y-3 p-4 text-center text-[13px] text-muted-foreground">
            <div>{t('chat.noSessions', 'No chat sessions yet')}</div>
            {loadError ? (
              <button
                type="button"
                className="inline-flex rounded-md border border-border/70 px-3 py-1.5 text-xs text-foreground transition-colors hover:bg-muted/60"
                onClick={() => void loadSessions()}
              >
                {t('common.retry', '重试')}
              </button>
            ) : null}
          </div>
        )}

        {timeFilter === 'all' ? (
          groups.map(group => (
            <div key={group.label}>
              <div className="sticky top-0 z-10 bg-muted/20 px-3 py-1 text-micro font-medium uppercase tracking-wider text-muted-foreground/75 backdrop-blur-sm">
                {group.label === 'pinned' ? '📌' : ''} {groupLabels[group.label] || group.label}
              </div>
              {group.items.map(session => (
                <SessionItem
                  key={session.session_id}
                  session={session}
                  isSelected={session.session_id === selectedSessionId}
                  isEditing={session.session_id === editingId}
                  editTitle={editTitle}
                  showMenu={session.session_id === menuSessionId}
                  onEditTitleChange={setEditTitle}
                  onSubmitRename={submitRename}
                  onCancelRename={cancelRename}
                  onClick={() => onSelectSession(session)}
                  onMenuToggle={(e) => {
                    e.stopPropagation();
                    setMenuSessionId(prev => prev === session.session_id ? null : session.session_id);
                  }}
                  onRename={() => handleRename(session.session_id)}
                  onPin={() => handlePin(session.session_id)}
                  onArchive={() => handleArchive(session.session_id)}
                  onDelete={() => handleDelete(session.session_id)}
                />
              ))}
            </div>
          ))
        ) : (
          filteredSessions.map(session => (
            <SessionItem
              key={session.session_id}
              session={session}
              isSelected={session.session_id === selectedSessionId}
              isEditing={session.session_id === editingId}
              editTitle={editTitle}
              showMenu={session.session_id === menuSessionId}
              onEditTitleChange={setEditTitle}
              onSubmitRename={submitRename}
              onCancelRename={cancelRename}
              onClick={() => onSelectSession(session)}
              onMenuToggle={(e) => {
                e.stopPropagation();
                setMenuSessionId(prev => prev === session.session_id ? null : session.session_id);
              }}
              onRename={() => handleRename(session.session_id)}
              onPin={() => handlePin(session.session_id)}
              onArchive={() => handleArchive(session.session_id)}
              onDelete={() => handleDelete(session.session_id)}
            />
          ))
        )}
        {sessions.length > 0 ? (
          <div className="px-3 pb-[calc(env(safe-area-inset-bottom)+12px)] pt-3">
            <div ref={sentinelRef} className="h-2 w-full" />
            <div className="rounded-md border border-dashed border-border/60 bg-muted/25 px-3 py-2 text-center text-[11px] text-muted-foreground">
              {isLoadingMore ? (
                <span className="inline-flex items-center gap-2">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t('chat.loadingMoreSessions', '加载更多会话')}
                </span>
              ) : loadError ? (
                <button
                  type="button"
                  className="inline-flex items-center gap-2 text-foreground hover:text-primary"
                  onClick={() => void loadMoreSessions()}
                >
                  {t('common.retry', '重试')}
                </button>
              ) : hasMore ? (
                <span>{t('chat.loadingMoreHint', '继续下滑可加载更多会话')}</span>
              ) : (
                <span>{t('chat.allSessionsLoaded', '已显示全部会话')}</span>
              )}
            </div>
          </div>
        ) : null}
      </div>
      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
        title={t('chat.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDelete}
      />
    </div>
  );
}

// --- Sub-components ---

function SessionItem({
  session,
  isSelected,
  isEditing,
  editTitle,
  showMenu,
  onEditTitleChange,
  onSubmitRename,
  onCancelRename,
  onClick,
  onMenuToggle,
  onRename,
  onPin,
  onArchive,
  onDelete,
}: {
  session: ChatSession;
  isSelected: boolean;
  isEditing: boolean;
  editTitle: string;
  showMenu: boolean;
  onEditTitleChange: (v: string) => void;
  onSubmitRename: () => void;
  onCancelRename: () => void;
  onClick: () => void;
  onMenuToggle: (e: React.MouseEvent) => void;
  onRename: () => void;
  onPin: () => void;
  onArchive: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const displayTitle = session.title || sanitizePreview(session.last_message_preview) || t('chat.newChat', 'New Chat');
  const timeStr = formatRelativeTime(session.last_message_at || session.created_at, t);
  const agentDisplay = session.agent_name && !isUUID(session.agent_name) ? session.agent_name : null;

  if (isEditing) {
    return (
      <div className="px-3 py-1.5">
        <input
          autoFocus
          value={editTitle}
          onChange={e => onEditTitleChange(e.target.value)}
          onBlur={onSubmitRename}
          onKeyDown={e => {
            if (e.key === 'Enter') onSubmitRename();
            if (e.key === 'Escape') {
              e.preventDefault();
              onCancelRename();
            }
          }}
          className="w-full text-[13px] rounded border px-2 py-1 bg-background"
        />
      </div>
    );
  }

  return (
    <div className="relative group">
      <button
        onClick={onClick}
        onContextMenu={(e) => { e.preventDefault(); onMenuToggle(e); }}
        className={`w-full text-left px-3 py-2 hover:bg-accent/50 transition-colors border-l-2 ${
          isSelected ? 'border-l-primary bg-accent/50' : 'border-l-transparent'
        }`}
      >
        {/* Line 1: title + fixed action slot */}
        <div className="flex items-center gap-1.5 min-w-0">
          {session.pinned && <Pin className="h-3 w-3 shrink-0 text-status-warning-text" />}
          <span className="truncate text-[13px] flex-1">{displayTitle}</span>
          {/* Reserve a stable right-side slot for the overflow menu button */}
          <span className="ml-auto h-5 w-5 shrink-0" aria-hidden />
        </div>
        {/* Line 2: time + agent tag + message count */}
        <div className="flex items-center gap-1.5 mt-0.5 min-w-0">
          <span className="shrink-0 text-micro text-muted-foreground/80">{timeStr}</span>
          {agentDisplay && (
            <span className="max-w-[7.5rem] truncate rounded bg-muted px-1.5 py-px text-micro text-muted-foreground/90 sm:max-w-[9rem]">
              {agentDisplay}
            </span>
          )}
          {session.message_count > 0 && (
            <span className="shrink-0 text-micro text-muted-foreground/75">
              {session.message_count} {t('chat.messagesShort', 'msgs')}
            </span>
          )}
        </div>
        {/* Line 3: message preview */}
        {session.last_message_preview && session.title && (
          <p className="mt-0.5 truncate text-caption text-muted-foreground/65">
            {sanitizePreview(session.last_message_preview)}
          </p>
        )}
      </button>

      {/* Hover action button */}
      <button
        onClick={onMenuToggle}
        className="absolute right-1.5 top-1.5 p-1 rounded hover:bg-muted opacity-0 group-hover:opacity-100 transition-opacity"
      >
        <MoreHorizontal className="h-3.5 w-3.5 text-muted-foreground" />
      </button>

      {/* Inline popover menu */}
      {showMenu && (
        <div
          className="absolute right-1 top-8 z-50 w-[min(160px,calc(100vw-1rem))] rounded-md border bg-popover py-1 shadow-md"
          onClick={e => e.stopPropagation()}
        >
          <button onClick={onRename} className="w-full px-3 py-1.5 text-xs text-left hover:bg-accent flex items-center gap-2">
            <Edit3 className="h-3 w-3" /> {t('chat.rename', 'Rename')}
          </button>
          <button onClick={onPin} className="w-full px-3 py-1.5 text-xs text-left hover:bg-accent flex items-center gap-2">
            <Pin className="h-3 w-3" /> {session.pinned ? t('chat.unpin', 'Unpin') : t('chat.pin', 'Pin')}
          </button>
          <button onClick={onArchive} className="w-full px-3 py-1.5 text-xs text-left hover:bg-accent flex items-center gap-2">
            <Archive className="h-3 w-3" /> {t('chat.archive', 'Archive')}
          </button>
          <div className="border-t my-0.5" />
          <button onClick={onDelete} className="w-full px-3 py-1.5 text-xs text-left hover:bg-accent text-destructive flex items-center gap-2">
            <Trash2 className="h-3 w-3" /> {t('common.delete', 'Delete')}
          </button>
        </div>
      )}
    </div>
  );
}
