import { useState, useEffect, useCallback, useMemo } from 'react';
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
  onSelectSession: (sessionId: string) => void;
  onSessionRemoved?: (sessionId: string) => void;
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

  const loadSessions = useCallback(async () => {
    setLoadingList(true);
    try {
      const list = await chatApi.listSessions(teamId, agentId, 1, 20, undefined, includeHidden);
      setSessions(list);
    } catch (e) {
      console.error('Failed to load sessions:', e);
    } finally {
      setLoadingList(false);
    }
  }, [teamId, agentId, includeHidden]);

  useEffect(() => {
    loadSessions();
  }, [loadSessions]);

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
      <div className="flex-1 overflow-y-auto scrollbar-subtle">
        {loadingList && sessions.length === 0 && (
          <div className="flex items-center justify-center p-4">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
          </div>
        )}
        {!loadingList && sessions.length === 0 && (
          <div className="p-4 text-center text-sm text-muted-foreground">
            {t('chat.noSessions', 'No chat sessions yet')}
          </div>
        )}

        {timeFilter === 'all' ? (
          groups.map(group => (
            <div key={group.label}>
              <div className="px-3 py-1 text-micro font-medium text-muted-foreground/60 uppercase tracking-wider sticky top-0 bg-muted/20 backdrop-blur-sm z-10">
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
                  onClick={() => onSelectSession(session.session_id)}
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
              onClick={() => onSelectSession(session.session_id)}
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
          className="w-full text-sm rounded border px-2 py-1 bg-background"
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
          {session.pinned && <Pin className="h-3 w-3 text-amber-500 shrink-0" />}
          <span className="truncate text-sm flex-1">{displayTitle}</span>
          {/* Reserve a stable right-side slot for the overflow menu button */}
          <span className="ml-auto h-5 w-5 shrink-0" aria-hidden />
        </div>
        {/* Line 2: time + agent tag + message count */}
        <div className="flex items-center gap-1.5 mt-0.5 min-w-0">
          <span className="text-micro text-muted-foreground/70 shrink-0">{timeStr}</span>
          {agentDisplay && (
            <span className="text-micro text-muted-foreground bg-muted rounded px-1.5 py-px truncate max-w-[120px]">
              {agentDisplay}
            </span>
          )}
          {session.message_count > 0 && (
            <span className="text-micro text-muted-foreground/60 shrink-0">
              {session.message_count} {t('chat.messagesShort', 'msgs')}
            </span>
          )}
        </div>
        {/* Line 3: message preview */}
        {session.last_message_preview && session.title && (
          <p className="text-caption text-muted-foreground/50 truncate mt-0.5">
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
          className="absolute right-1 top-8 z-50 bg-popover border rounded-md shadow-md py-1 min-w-[130px]"
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
