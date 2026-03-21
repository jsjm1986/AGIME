import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../ui/card';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog';
import { Input } from '../ui/input';
import { Pagination } from '../ui/pagination';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../ui/table';
import {
  ArrowLeft,
  ChevronRight,
  FileText,
  History,
  KeyRound,
  MessageSquareText,
  RefreshCw,
  Search,
  ShieldCheck,
  ShieldOff,
  SlidersHorizontal,
  UserRound,
  Users,
} from 'lucide-react';
import { useToast } from '../../contexts/ToastContext';
import {
  externalUsersApi,
  type ExternalUserDetail,
  type ExternalUserEventResponse,
  type ExternalUserStatus,
  type ExternalUserSummary,
} from '../../api/externalUsers';
import { formatDateTime } from '../../utils/format';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { ManagementRail } from '../mobile/ManagementRail';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';

interface ExternalUsersTabProps {
  teamId: string;
}

function statusVariant(status: ExternalUserStatus) {
  return status === 'active' ? 'default' as const : 'secondary' as const;
}

export function ExternalUsersTab({ teamId }: ExternalUsersTabProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();
  const { addToast } = useToast();
  const isConversationTaskMode = isConversationMode && isMobileWorkspace;

  const [users, setUsers] = useState<ExternalUserSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(1);
  const [totalUsers, setTotalUsers] = useState(0);
  const [searchInput, setSearchInput] = useState('');
  const [searchTerm, setSearchTerm] = useState('');
  const [statusFilter, setStatusFilter] = useState<'all' | ExternalUserStatus>('all');
  const [selectedUserId, setSelectedUserId] = useState<string | null>(null);
  const [detail, setDetail] = useState<ExternalUserDetail | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [events, setEvents] = useState<ExternalUserEventResponse[]>([]);
  const [eventsLoading, setEventsLoading] = useState(false);
  const [actionLoadingId, setActionLoadingId] = useState<string | null>(null);
  const [resetTarget, setResetTarget] = useState<ExternalUserSummary | null>(null);
  const [newPassword, setNewPassword] = useState('');
  const [resetSaving, setResetSaving] = useState(false);
  const [mobileFilterSheetOpen, setMobileFilterSheetOpen] = useState(false);
  const [mobileView, setMobileView] = useState<'list' | 'detail'>('list');
  const [activeMobilePanel, setActiveMobilePanel] = useState<null | 'filters' | 'events' | 'reset'>(null);
  const mobileSearchInputRef = useRef<HTMLInputElement | null>(null);
  const isMobileLayout = isMobileWorkspace;

  const selectedSummary = useMemo(
    () => users.find((user) => user.id === selectedUserId) ?? detail?.user ?? null,
    [detail?.user, selectedUserId, users],
  );

  const openMobileDetail = (userId: string) => {
    setSelectedUserId(userId);
    setMobileView('detail');
  };

  const closeMobilePanels = () => {
    setActiveMobilePanel(null);
    setMobileFilterSheetOpen(false);
  };

  const closeResetFlow = () => {
    setResetTarget(null);
    setNewPassword('');
    setActiveMobilePanel(null);
  };

  const focusMobileSearch = () => {
    setMobileView('list');
    window.setTimeout(() => {
      mobileSearchInputRef.current?.focus();
    }, 0);
  };

  const loadUsers = async () => {
    try {
      setLoading(true);
      const response = await externalUsersApi.listUsers(teamId, {
        page,
        limit: 12,
        search: searchTerm || undefined,
        status: statusFilter,
      });
      setUsers(response.items);
      setTotalPages(response.totalPages || 1);
      setTotalUsers(response.total);
      setError('');
      if (response.items.length === 0) {
        setSelectedUserId(null);
        setDetail(null);
        setEvents([]);
      } else if (!selectedUserId || !response.items.some((item) => item.id === selectedUserId)) {
        setSelectedUserId(response.items[0].id);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void loadUsers();
  }, [teamId, page, searchTerm, statusFilter]);

  useEffect(() => {
    if (!selectedUserId) return;
    let cancelled = false;

    const loadDetail = async () => {
      try {
        setDetailLoading(true);
        const [detailResponse, eventsResponse] = await Promise.all([
          externalUsersApi.getUserDetail(teamId, selectedUserId),
          externalUsersApi.listEvents(teamId, {
            externalUserId: selectedUserId,
            page: 1,
            limit: 12,
          }),
        ]);
        if (cancelled) return;
        setDetail(detailResponse);
        setEvents(eventsResponse.items);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : t('common.error'));
        }
      } finally {
        if (!cancelled) {
          setDetailLoading(false);
          setEventsLoading(false);
        }
      }
    };

    setEventsLoading(true);
    void loadDetail();
    return () => {
      cancelled = true;
    };
  }, [selectedUserId, teamId, t]);

  useEffect(() => {
    if (!isMobileLayout) return;
    if (mobileView === 'detail' && !selectedSummary) {
      setMobileView('list');
    }
  }, [isMobileLayout, mobileView, selectedSummary]);

  const applySearch = () => {
    setPage(1);
    setSearchTerm(searchInput.trim());
    closeMobilePanels();
  };

  const handleToggleStatus = async (user: ExternalUserSummary) => {
    try {
      setActionLoadingId(user.id);
      if (user.status === 'active') {
        await externalUsersApi.disable(teamId, user.id);
        addToast('success', t('teamAdmin.externalUsers.disabled', '已禁用外部用户'));
      } else {
        await externalUsersApi.enable(teamId, user.id);
        addToast('success', t('teamAdmin.externalUsers.enabled', '已启用外部用户'));
      }
      await loadUsers();
      if (selectedUserId === user.id) {
        const refreshed = await externalUsersApi.getUserDetail(teamId, user.id);
        setDetail(refreshed);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setActionLoadingId(null);
    }
  };

  const handleResetPassword = async () => {
    if (!resetTarget || !newPassword.trim()) return;
    try {
      setResetSaving(true);
      await externalUsersApi.resetPassword(teamId, resetTarget.id, newPassword.trim());
      addToast('success', t('teamAdmin.externalUsers.passwordReset', '密码已重置'));
      setResetTarget(null);
      setNewPassword('');
      setActiveMobilePanel(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setResetSaving(false);
    }
  };

  const filterControls = (
    <div className="space-y-3">
      <Input
        value={searchInput}
        onChange={(event) => setSearchInput(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            applySearch();
          }
        }}
        placeholder={t('teamAdmin.externalUsers.search', '按用户名、显示名称或手机号搜索')}
      />
      <Select
        value={statusFilter}
        onValueChange={(value) => {
          setPage(1);
          setStatusFilter(value as 'all' | ExternalUserStatus);
        }}
      >
        <SelectTrigger className="h-10">
          <SelectValue placeholder={t('teamAdmin.externalUsers.statusAll', '全部状态')} />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="all">{t('teamAdmin.externalUsers.statusAll', '全部状态')}</SelectItem>
          <SelectItem value="active">{t('teamAdmin.externalUsers.statusActive', '启用中')}</SelectItem>
          <SelectItem value="disabled">{t('teamAdmin.externalUsers.statusDisabled', '已禁用')}</SelectItem>
        </SelectContent>
      </Select>
      <div className="flex gap-2">
        <Button variant="outline" className="flex-1" onClick={applySearch}>
          {t('common.search', '搜索')}
        </Button>
        <Button variant="outline" className="flex-1" onClick={() => void loadUsers()}>
          {t('common.refresh', '刷新')}
        </Button>
      </div>
    </div>
  );

  const desktopResetPasswordDialog = (
    <Dialog open={!!resetTarget && !isMobileLayout} onOpenChange={(open) => {
      if (!open) {
        closeResetFlow();
      }
    }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('teamAdmin.externalUsers.resetPassword', '重置密码')}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3 py-2">
          <p className="text-sm text-[hsl(var(--muted-foreground))]">
            {resetTarget
              ? t('teamAdmin.externalUsers.resetPasswordHint', '为 {{name}} 设置一个新密码。当前用户的已有登录 session 会被清理。', {
                  name: resetTarget.displayName || resetTarget.username,
                })
              : ''}
          </p>
          <Input
            type="password"
            value={newPassword}
            onChange={(event) => setNewPassword(event.target.value)}
            placeholder={t('teamAdmin.externalUsers.newPassword', '请输入新密码')}
          />
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => {
              closeResetFlow();
            }}
          >
            {t('common.cancel')}
          </Button>
          <Button onClick={() => void handleResetPassword()} disabled={resetSaving || !newPassword.trim()}>
            {resetSaving ? t('common.saving') : t('common.confirm', '确认')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );

  const statusLabel = (status: ExternalUserStatus) => (
    status === 'active'
      ? t('teamAdmin.externalUsers.statusActive', '启用中')
      : t('teamAdmin.externalUsers.statusDisabled', '已禁用')
  );

  const mobileSummaryLine = selectedSummary
    ? `${selectedSummary.displayName || selectedSummary.username} · ${statusLabel(selectedSummary.status)}`
    : t('teamAdmin.externalUsers.mobileNoSelection', '先从列表中选择一个用户');

  const formatUserLastSeen = (user: ExternalUserSummary) => (
    user.lastSeenAt
      ? formatDateTime(user.lastSeenAt)
      : (user.lastLoginAt ? formatDateTime(user.lastLoginAt) : '—')
  );

  const renderDetailField = (label: string, value: string) => (
    <div className="flex items-center justify-between gap-3 text-[12px] leading-5">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate text-right font-medium text-foreground">{value || '—'}</span>
    </div>
  );

  const renderUploadsList = (compact = false) => {
    if (!detail || detail.recentUploads.length === 0) {
      return (
        <p className="text-sm text-[hsl(var(--muted-foreground))]">
          {t('teamAdmin.externalUsers.noUploads', '暂无上传记录')}
        </p>
      );
    }

    return (
      <div className={compact ? 'space-y-2' : 'space-y-2.5'}>
        {detail.recentUploads.map((doc) => (
          <div
            key={doc.id}
            className="rounded-[16px] border border-border/70 bg-card px-3 py-2.5"
          >
            <div className="truncate text-[13px] font-medium text-foreground">
              {doc.display_name || doc.name}
            </div>
            <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
              <span>{doc.mime_type || t('common.unknown', '未知')}</span>
              <span>·</span>
              <span>{formatDateTime(doc.updated_at || doc.created_at)}</span>
            </div>
          </div>
        ))}
      </div>
    );
  };

  const renderSessionsList = (compact = false) => {
    if (!detail || detail.recentSessions.length === 0) {
      return (
        <p className="text-sm text-[hsl(var(--muted-foreground))]">
          {t('teamAdmin.externalUsers.noSessions', '暂无会话记录')}
        </p>
      );
    }

    return (
      <div className={compact ? 'space-y-2' : 'space-y-2.5'}>
        {detail.recentSessions.map((session) => (
          <div
            key={session.sessionId}
            className="rounded-[16px] border border-border/70 bg-card px-3 py-2.5"
          >
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="truncate text-[13px] font-medium text-foreground">
                  {session.title || session.portalSlug || session.sessionId}
                </div>
                <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
                  <span>{session.portalSlug || 'portal'}</span>
                  <span>·</span>
                  <span>{formatDateTime(session.updatedAt)}</span>
                  <span>·</span>
                  <span>{t('teamAdmin.externalUsers.messageCount', '{{count}} 条消息', { count: session.messageCount })}</span>
                </div>
              </div>
              {session.isProcessing ? (
                <Badge variant="outline">{t('teamAdmin.externalUsers.processing', '处理中')}</Badge>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    );
  };

  const renderEventsList = () => {
    if (eventsLoading) {
      return (
        <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
          {t('common.loading')}
        </p>
      );
    }

    if (events.length === 0) {
      return (
        <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
          {t('teamAdmin.externalUsers.noEvents', '暂无事件记录')}
        </p>
      );
    }

    return (
      <div className="space-y-2.5">
        {events.map((event) => (
          <div
            key={event.id}
            className="rounded-[16px] border border-border/70 bg-card px-3 py-2.5"
          >
            <div className="flex items-center justify-between gap-3">
              <div className="text-[13px] font-medium text-foreground">{event.eventType}</div>
              <Badge variant={event.result === 'success' ? 'default' : 'secondary'}>
                {event.result}
              </Badge>
            </div>
            <div className="mt-1 flex flex-wrap gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
              <span>{formatDateTime(event.createdAt)}</span>
              {event.portalSlug ? (
                <>
                  <span>·</span>
                  <span>{event.portalSlug}</span>
                </>
              ) : null}
              {event.visitorId ? (
                <>
                  <span>·</span>
                  <span>{`visitor ${event.visitorId}`}</span>
                </>
              ) : null}
            </div>
          </div>
        ))}
      </div>
    );
  };

  const renderMobileListItem = (user: ExternalUserSummary) => (
    <div
      key={user.id}
      className="rounded-[18px] border border-border/65 bg-card px-3.5 py-3 shadow-[0_8px_18px_hsl(var(--ui-shadow))/0.025]"
    >
      <div className="flex items-start gap-3">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[14px] border border-border/65 bg-background text-muted-foreground">
          <UserRound className="h-4.5 w-4.5" />
        </div>
        <button
          type="button"
          onClick={() => openMobileDetail(user.id)}
          className="min-w-0 flex-1 text-left"
        >
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              <div className="truncate text-[14px] font-semibold leading-5 tracking-[-0.015em] text-foreground">
                {user.displayName || user.username}
              </div>
              <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                {user.username}
              </div>
            </div>
            <Badge
              variant={statusVariant(user.status)}
              className="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium"
            >
              {statusLabel(user.status)}
            </Badge>
          </div>
          <div className="mt-1.5 flex flex-wrap gap-x-2 gap-y-1 text-[11px] text-muted-foreground">
            <span>{user.phone || t('teamAdmin.externalUsers.noPhone', '未填写手机号')}</span>
            <span>·</span>
            <span>{formatUserLastSeen(user)}</span>
          </div>
          <div className="mt-1.5 text-[11px] text-muted-foreground">
            {t('teamAdmin.externalUsers.uploadCountBadge', '{{count}} 上传', { count: user.uploadCount })}
            {' · '}
            {t('teamAdmin.externalUsers.sessionCountBadge', '{{count}} 会话', { count: user.sessionCount })}
            {' · '}
            {t('teamAdmin.externalUsers.eventCountBadge', '{{count}} 事件', { count: user.eventCount })}
          </div>
        </button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="mt-0.5 h-8 w-8 shrink-0 rounded-full p-0 text-muted-foreground"
          onClick={() => openMobileDetail(user.id)}
        >
          <ChevronRight className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );

  const renderMobileListToolbar = () => (
    <div className="border-b border-border/55 bg-card px-3.5 pb-3 pt-3">
      <div className="rounded-[20px] border border-border/60 bg-background px-3 py-3 shadow-[0_8px_18px_hsl(var(--ui-shadow))/0.02]">
        <div className="flex gap-2">
          <div className="relative flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              ref={mobileSearchInputRef}
              value={searchInput}
              onChange={(event) => setSearchInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  applySearch();
                }
              }}
              placeholder={t('teamAdmin.externalUsers.search', '按用户名、显示名称或手机号搜索')}
              className="h-11 rounded-[15px] border-border/60 bg-card pl-10 text-[13px]"
            />
          </div>
          <Button className="h-11 rounded-[15px] px-4 text-[12px]" onClick={applySearch}>
            {t('common.search', '搜索')}
          </Button>
        </div>
        <div className="mt-2 flex gap-2">
          <Select
            value={statusFilter}
            onValueChange={(value) => {
              setPage(1);
              setStatusFilter(value as 'all' | ExternalUserStatus);
            }}
          >
            <SelectTrigger className="h-10 flex-1 rounded-[14px] border-border/60 bg-card text-[12px]">
              <SelectValue placeholder={t('teamAdmin.externalUsers.statusAll', '全部状态')} />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t('teamAdmin.externalUsers.statusAll', '全部状态')}</SelectItem>
              <SelectItem value="active">{t('teamAdmin.externalUsers.statusActive', '启用中')}</SelectItem>
              <SelectItem value="disabled">{t('teamAdmin.externalUsers.statusDisabled', '已禁用')}</SelectItem>
            </SelectContent>
          </Select>
          <Button
            variant="outline"
            className="h-10 rounded-[14px] border-border/60 px-3 text-[12px]"
            onClick={() => {
              setMobileFilterSheetOpen(true);
              setActiveMobilePanel('filters');
            }}
          >
            <SlidersHorizontal className="mr-1.5 h-4 w-4" />
            {t('teamAdmin.externalUsers.quickFilters', '筛选')}
          </Button>
          <Button
            variant="ghost"
            className="h-10 w-10 shrink-0 rounded-[14px] p-0 text-muted-foreground"
            onClick={() => void loadUsers()}
          >
            <RefreshCw className="h-4 w-4" />
          </Button>
        </div>
        <div className="mt-3 flex items-center justify-between gap-3 rounded-[16px] border border-border/55 bg-card px-3 py-2.5">
          <div className="min-w-0">
            <div className="truncate text-[12px] font-semibold text-foreground">
              {(statusFilter === 'all'
                ? t('teamAdmin.externalUsers.statusAll', '全部状态')
                : statusLabel(statusFilter))}
              {' · '}
              {t('teamAdmin.externalUsers.summaryTotal', '用户总数')} {totalUsers}
            </div>
            <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
              {searchTerm
                ? t('teamAdmin.externalUsers.searchApplied', '搜索：{{term}}', { term: searchTerm })
                : mobileSummaryLine}
            </div>
          </div>
          {selectedSummary ? (
            <Button
              size="sm"
              variant="outline"
              className="h-8 rounded-full border-border/60 px-3 text-[11px]"
              onClick={() => setMobileView('detail')}
            >
              {t('common.view', '查看')}
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );

  const renderMobilePagination = () => (
    totalPages > 1 ? (
      <div className="border-t border-border/55 px-3.5 py-2.5">
        <div className="flex items-center justify-between gap-3 text-[11px] text-muted-foreground">
          <div className="min-w-0 truncate">
            {t('common.pageLabel', '第 {{page}} / {{total}} 页', { page, total: totalPages })}
            {' · '}
            {t('teamAdmin.externalUsers.paginationHint', '共 {{count}} 位用户', { count: totalUsers })}
          </div>
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="outline"
              className="h-8 rounded-full border-border/60 px-3 text-[11px]"
              disabled={page <= 1}
              onClick={() => setPage((current) => Math.max(1, current - 1))}
            >
              {t('common.previous', '上一页')}
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-8 rounded-full border-border/60 px-3 text-[11px]"
              disabled={page >= totalPages}
              onClick={() => setPage((current) => Math.min(totalPages, current + 1))}
            >
              {t('common.next', '下一页')}
            </Button>
          </div>
        </div>
      </div>
    ) : null
  );

  const renderMobileListView = (forShell = false) => (
    <div className={`flex min-h-0 flex-1 flex-col ${forShell ? '' : 'rounded-[24px] border border-border/65 bg-card shadow-[0_12px_24px_hsl(var(--ui-shadow))/0.035]'}`}>
      {renderMobileListToolbar()}
      {error ? (
        <div className="border-b border-border/60 px-3.5 py-2 text-[12px] text-destructive">
          {error}
        </div>
      ) : null}
      <div className={`${forShell ? 'min-h-0 flex-1 overflow-y-auto' : ''} px-3.5 py-3`}>
        {loading ? (
          <p className="py-10 text-center text-sm text-[hsl(var(--muted-foreground))]">
            {t('common.loading')}
          </p>
        ) : users.length === 0 ? (
          <div className="rounded-[18px] border border-dashed border-border/70 bg-background px-4 py-10 text-center text-sm text-muted-foreground">
            {t('teamAdmin.externalUsers.empty', '暂无匹配的外部用户')}
          </div>
        ) : (
          <div className="space-y-2">
            {users.map(renderMobileListItem)}
          </div>
        )}
      </div>
      {renderMobilePagination()}
    </div>
  );

  const renderMobileDetailView = (forShell = false) => (
    <div className={`flex min-h-0 flex-1 flex-col ${forShell ? '' : 'rounded-[24px] border border-border/65 bg-card shadow-[0_12px_24px_hsl(var(--ui-shadow))/0.035]'}`}>
      <div className="border-b border-border/55 bg-card px-3.5 py-3">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 items-start gap-2.5">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="mt-0.5 h-8 shrink-0 rounded-full px-2.5 text-[11px] text-muted-foreground"
              onClick={() => setMobileView('list')}
            >
              <ArrowLeft className="mr-1.5 h-3.5 w-3.5" />
              {t('common.back', '返回')}
            </Button>
            <div className="min-w-0">
              <div className="truncate text-[15px] font-semibold tracking-[-0.02em] text-foreground">
                {selectedSummary?.displayName || selectedSummary?.username || t('teamAdmin.externalUsers.detailTitle', '用户详情')}
              </div>
              <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                {selectedSummary?.username || t('teamAdmin.externalUsers.selectUser', '从列表选择一个用户')}
              </div>
            </div>
          </div>
          {selectedSummary ? (
            <Badge variant={statusVariant(selectedSummary.status)}>{statusLabel(selectedSummary.status)}</Badge>
          ) : null}
        </div>
      </div>

      <div className={`${forShell ? 'min-h-0 flex-1 overflow-y-auto' : ''} space-y-3 px-3.5 py-3`}>
        {detailLoading ? (
          <p className="py-10 text-center text-sm text-[hsl(var(--muted-foreground))]">
            {t('common.loading')}
          </p>
        ) : !detail || !selectedSummary ? (
          <div className="rounded-[18px] border border-dashed border-border/70 bg-background px-4 py-10 text-center text-sm text-muted-foreground">
            {t('teamAdmin.externalUsers.selectUser', '从列表选择一个外部用户查看详情。')}
          </div>
        ) : (
          <>
            <div className="rounded-[18px] border border-border/65 bg-card px-3.5 py-3">
              <div className="flex items-start gap-3">
                <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-[16px] border border-border/60 bg-background text-muted-foreground">
                  <UserRound className="h-5 w-5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[16px] font-semibold tracking-[-0.02em] text-foreground">
                    {selectedSummary.displayName || selectedSummary.username}
                  </div>
                  <div className="mt-0.5 truncate text-[12px] text-muted-foreground">
                    {selectedSummary.username}
                  </div>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <Badge variant="outline">{t('teamAdmin.externalUsers.uploadCountBadge', '{{count}} 上传', { count: selectedSummary.uploadCount })}</Badge>
                    <Badge variant="outline">{t('teamAdmin.externalUsers.sessionCountBadge', '{{count}} 会话', { count: selectedSummary.sessionCount })}</Badge>
                    <Badge variant="outline">{t('teamAdmin.externalUsers.eventCountBadge', '{{count}} 事件', { count: selectedSummary.eventCount })}</Badge>
                  </div>
                </div>
              </div>
              <div className="mt-3 space-y-2 rounded-[16px] border border-border/60 bg-background px-3 py-2.5">
                {renderDetailField(t('teamAdmin.externalUsers.detailPhone', '手机号'), selectedSummary.phone || t('teamAdmin.externalUsers.noPhone', '未填写手机号'))}
                {renderDetailField(t('teamAdmin.externalUsers.detailCreated', '创建时间'), formatDateTime(selectedSummary.createdAt))}
                {renderDetailField(t('teamAdmin.externalUsers.detailLastLogin', '最近登录'), selectedSummary.lastLoginAt ? formatDateTime(selectedSummary.lastLoginAt) : '—')}
                {renderDetailField(t('teamAdmin.externalUsers.detailLastSeen', '最近活跃'), selectedSummary.lastSeenAt ? formatDateTime(selectedSummary.lastSeenAt) : '—')}
              </div>
            </div>

            <div className="grid grid-cols-2 gap-2">
              <Button
                variant="outline"
                className="h-11 justify-start rounded-[16px] border-border/60"
                onClick={() => void handleToggleStatus(selectedSummary)}
                disabled={actionLoadingId === selectedSummary.id}
              >
                {selectedSummary.status === 'active' ? (
                  <ShieldOff className="mr-2 h-4 w-4" />
                ) : (
                  <ShieldCheck className="mr-2 h-4 w-4" />
                )}
                {selectedSummary.status === 'active'
                  ? t('teamAdmin.externalUsers.disable', '禁用')
                  : t('teamAdmin.externalUsers.enable', '启用')}
              </Button>
              <Button
                variant="outline"
                className="h-11 justify-start rounded-[16px] border-border/60"
                onClick={() => {
                  setResetTarget(selectedSummary);
                  setNewPassword('');
                  setActiveMobilePanel('reset');
                }}
              >
                <KeyRound className="mr-2 h-4 w-4" />
                {t('teamAdmin.externalUsers.resetPassword', '重置密码')}
              </Button>
            </div>

            <div className="rounded-[18px] border border-border/65 bg-card px-3.5 py-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-[13px] font-semibold text-foreground">
                    {t('teamAdmin.externalUsers.linkedVisitors', '关联访客标识')}
                  </div>
                  <div className="mt-1 text-[11px] text-muted-foreground">
                    {t('teamAdmin.externalUsers.linkedVisitorsHint', '访客标识可帮助追踪外部用户来源。')}
                  </div>
                </div>
                <div className="flex h-9 w-9 items-center justify-center rounded-full border border-border/70 bg-background text-muted-foreground">
                  <Users className="h-4 w-4" />
                </div>
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                {detail.linkedVisitorIds.length === 0 ? (
                  <span className="text-sm text-muted-foreground">—</span>
                ) : (
                  detail.linkedVisitorIds.map((visitorId) => (
                    <Badge key={visitorId} variant="outline">{visitorId}</Badge>
                  ))
                )}
              </div>
            </div>

            <div className="rounded-[18px] border border-border/65 bg-card px-3.5 py-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div className="text-[13px] font-semibold text-foreground">
                  {t('teamAdmin.externalUsers.recentUploads', '最近上传')}
                </div>
                <FileText className="h-4 w-4 text-muted-foreground" />
              </div>
              {renderUploadsList(true)}
            </div>

            <div className="rounded-[18px] border border-border/65 bg-card px-3.5 py-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div className="text-[13px] font-semibold text-foreground">
                  {t('teamAdmin.externalUsers.recentSessions', '最近会话')}
                </div>
                <History className="h-4 w-4 text-muted-foreground" />
              </div>
              {renderSessionsList(true)}
            </div>

            <div className="rounded-[18px] border border-border/65 bg-card px-3.5 py-3">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-[13px] font-semibold text-foreground">
                    {t('teamAdmin.externalUsers.eventsTitle', '最近事件')}
                  </div>
                  <div className="mt-1 text-[11px] text-muted-foreground">
                    {t('teamAdmin.externalUsers.eventsDesc', '注册、登录、访客绑定等关键动作会记录在这里。')}
                  </div>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-9 rounded-full border-border/60 px-3"
                  onClick={() => setActiveMobilePanel('events')}
                >
                  {t('teamAdmin.externalUsers.viewEvents', '查看')}
                </Button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );

  const mobileFilterPanel = (
    <BottomSheetPanel
      open={activeMobilePanel === 'filters' || mobileFilterSheetOpen}
      onOpenChange={(open) => {
        if (!open) {
          closeMobilePanels();
        }
      }}
      title={t('teamAdmin.externalUsers.quickFilters', '筛选与搜索')}
      description={t('teamAdmin.externalUsers.filterHint', '调整搜索词和状态筛选，快速定位目标外部用户。')}
      onBack={closeMobilePanels}
      hideCloseButton
    >
      <div className="space-y-4">
        {filterControls}
      </div>
    </BottomSheetPanel>
  );

  const mobileEventsPanel = (
    <BottomSheetPanel
      open={activeMobilePanel === 'events'}
      onOpenChange={(open) => {
        if (!open) {
          setActiveMobilePanel(null);
        }
      }}
      title={t('teamAdmin.externalUsers.eventsTitle', '最近事件')}
      description={selectedSummary ? `${selectedSummary.displayName || selectedSummary.username} · ${statusLabel(selectedSummary.status)}` : t('teamAdmin.externalUsers.selectUser', '从列表选择一个外部用户查看详情。')}
      onBack={() => setActiveMobilePanel(null)}
      hideCloseButton
    >
      {renderEventsList()}
    </BottomSheetPanel>
  );

  const mobileResetPasswordPanel = (
    <BottomSheetPanel
      open={activeMobilePanel === 'reset'}
      onOpenChange={(open) => {
        if (!open) {
          closeResetFlow();
        }
      }}
      title={t('teamAdmin.externalUsers.resetPassword', '重置密码')}
      description={resetTarget
        ? t('teamAdmin.externalUsers.resetPasswordHint', '为 {{name}} 设置一个新密码。当前用户的已有登录 session 会被清理。', {
            name: resetTarget.displayName || resetTarget.username,
          })
        : ''}
      onBack={closeResetFlow}
      hideCloseButton
    >
      <div className="space-y-4">
        <Input
          type="password"
          value={newPassword}
          onChange={(event) => setNewPassword(event.target.value)}
          placeholder={t('teamAdmin.externalUsers.newPassword', '请输入新密码')}
          className="h-11 rounded-[16px]"
        />
        <div className="flex gap-2">
          <Button variant="outline" className="flex-1 rounded-[16px]" onClick={closeResetFlow}>
            {t('common.cancel')}
          </Button>
          <Button className="flex-1 rounded-[16px]" onClick={() => void handleResetPassword()} disabled={resetSaving || !newPassword.trim()}>
            {resetSaving ? t('common.saving') : t('common.confirm', '确认')}
          </Button>
        </div>
      </div>
    </BottomSheetPanel>
  );

  const externalUsersContent = (
    <>
      <div className="space-y-4">
        <Card>
          <CardHeader className="pb-4">
            <CardTitle className="text-lg">{t('teamAdmin.externalUsers.title', '外部用户')}</CardTitle>
            <CardDescription>
              {t(
                'teamAdmin.externalUsers.description',
                '查看分身对外注册用户、上传资料归属、最近会话与访问事件。'
              )}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
              <Input
                value={searchInput}
                onChange={(event) => setSearchInput(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    applySearch();
                  }
                }}
                placeholder={t('teamAdmin.externalUsers.search', '按用户名、显示名称或手机号搜索')}
                className="lg:max-w-md"
              />
              <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
                <Select
                  value={statusFilter}
                  onValueChange={(value) => {
                    setPage(1);
                    setStatusFilter(value as 'all' | ExternalUserStatus);
                  }}
                >
                  <SelectTrigger className="w-full sm:w-[min(180px,100%)]">
                    <SelectValue placeholder={t('teamAdmin.externalUsers.statusAll', '全部状态')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t('teamAdmin.externalUsers.statusAll', '全部状态')}</SelectItem>
                    <SelectItem value="active">{t('teamAdmin.externalUsers.statusActive', '启用中')}</SelectItem>
                    <SelectItem value="disabled">{t('teamAdmin.externalUsers.statusDisabled', '已禁用')}</SelectItem>
                  </SelectContent>
                </Select>
                <Button variant="outline" onClick={applySearch}>
                  {t('common.search', '搜索')}
                </Button>
                <Button variant="outline" onClick={() => void loadUsers()}>
                  {t('common.refresh', '刷新')}
                </Button>
              </div>
            </div>

            {error && (
              <div className="rounded-md border border-[hsl(var(--destructive))]/30 bg-[hsl(var(--destructive))]/5 px-3 py-2 text-sm text-[hsl(var(--destructive))]">
                {error}
              </div>
            )}

            <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(360px,0.8fr)]">
              <Card>
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">
                    {t('teamAdmin.externalUsers.listTitle', '用户列表')}
                  </CardTitle>
                  <CardDescription>
                    {t('teamAdmin.externalUsers.listDesc', '按当前团队下的外部注册用户查看。')}
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  {loading ? (
                    <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                      {t('common.loading')}
                    </p>
                  ) : users.length === 0 ? (
                    <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                      {t('teamAdmin.externalUsers.empty', '当前还没有外部用户。')}
                    </p>
                  ) : (
                    <>
                      <Table>
                        <TableHeader>
                          <TableRow>
                            <TableHead>{t('teamAdmin.externalUsers.username', '用户名')}</TableHead>
                            <TableHead>{t('teamAdmin.externalUsers.status', '状态')}</TableHead>
                            <TableHead>{t('teamAdmin.externalUsers.usage', '使用情况')}</TableHead>
                            <TableHead>{t('teamAdmin.externalUsers.lastSeen', '最近活跃')}</TableHead>
                            <TableHead className="w-[160px]">{t('common.actions')}</TableHead>
                          </TableRow>
                        </TableHeader>
                        <TableBody>
                          {users.map((user) => {
                            const selected = user.id === selectedUserId;
                            return (
                              <TableRow
                                key={user.id}
                                className={selected ? 'bg-[hsl(var(--muted))]/50' : undefined}
                              >
                                <TableCell>
                                  <button
                                    type="button"
                                    className="text-left"
                                    onClick={() => setSelectedUserId(user.id)}
                                  >
                                    <div className="font-medium">{user.displayName || user.username}</div>
                                    <div className="text-xs text-[hsl(var(--muted-foreground))]">
                                      {user.username}
                                      {user.phone ? ` · ${user.phone}` : ''}
                                    </div>
                                  </button>
                                </TableCell>
                                <TableCell>
                                  <Badge variant={statusVariant(user.status)}>
                                    {user.status === 'active'
                                      ? t('teamAdmin.externalUsers.statusActive', '启用中')
                                      : t('teamAdmin.externalUsers.statusDisabled', '已禁用')}
                                  </Badge>
                                </TableCell>
                                <TableCell className="text-xs text-[hsl(var(--muted-foreground))]">
                                  <div>{t('teamAdmin.externalUsers.uploadCount', '上传 {{count}} 份', { count: user.uploadCount })}</div>
                                  <div>{t('teamAdmin.externalUsers.sessionCount', '会话 {{count}} 个', { count: user.sessionCount })}</div>
                                  <div>{t('teamAdmin.externalUsers.visitorCount', '关联访客 {{count}} 个', { count: user.linkedVisitorCount })}</div>
                                </TableCell>
                                <TableCell className="text-xs text-[hsl(var(--muted-foreground))]">
                                  {user.lastSeenAt ? formatDateTime(user.lastSeenAt) : '—'}
                                </TableCell>
                                <TableCell>
                                  <div className="flex flex-wrap gap-2">
                                    <Button
                                      size="sm"
                                      variant="outline"
                                      onClick={() => setSelectedUserId(user.id)}
                                    >
                                      {t('common.view', '查看')}
                                    </Button>
                                    <Button
                                      size="sm"
                                      variant="outline"
                                      disabled={actionLoadingId === user.id}
                                      onClick={() => void handleToggleStatus(user)}
                                    >
                                      {user.status === 'active'
                                        ? t('teamAdmin.externalUsers.disable', '禁用')
                                        : t('teamAdmin.externalUsers.enable', '启用')}
                                    </Button>
                                    <Button
                                      size="sm"
                                      variant="outline"
                                      onClick={() => {
                                        setResetTarget(user);
                                        setNewPassword('');
                                      }}
                                    >
                                      {t('teamAdmin.externalUsers.resetPassword', '重置密码')}
                                    </Button>
                                  </div>
                                </TableCell>
                              </TableRow>
                            );
                          })}
                        </TableBody>
                      </Table>
                      <Pagination
                        currentPage={page}
                        totalPages={totalPages}
                        totalItems={totalUsers}
                        pageSize={12}
                        onPageChange={setPage}
                      />
                    </>
                  )}
                </CardContent>
              </Card>

              <div className="space-y-4">
                <Card>
                  <CardHeader className="pb-2">
                    <CardTitle className="text-base">
                      {t('teamAdmin.externalUsers.detailTitle', '用户详情')}
                    </CardTitle>
                    <CardDescription>
                      {t('teamAdmin.externalUsers.detailDesc', '查看当前用户的绑定访客、最近上传和最近会话。')}
                    </CardDescription>
                  </CardHeader>
                  <CardContent>
                    {detailLoading ? (
                      <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                        {t('common.loading')}
                      </p>
                    ) : !detail || !selectedSummary ? (
                      <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                        {t('teamAdmin.externalUsers.selectUser', '从左侧选择一个外部用户查看详情。')}
                      </p>
                    ) : (
                      <div className="space-y-5">
                        <div className="space-y-2">
                          <div className="flex items-center justify-between gap-3">
                            <div>
                              <h3 className="text-lg font-semibold">{selectedSummary.displayName || selectedSummary.username}</h3>
                              <p className="text-sm text-[hsl(var(--muted-foreground))]">{selectedSummary.username}</p>
                            </div>
                            <Badge variant={statusVariant(selectedSummary.status)}>
                              {selectedSummary.status === 'active'
                                ? t('teamAdmin.externalUsers.statusActive', '启用中')
                                : t('teamAdmin.externalUsers.statusDisabled', '已禁用')}
                            </Badge>
                          </div>
                          <div className="grid gap-2 text-sm text-[hsl(var(--muted-foreground))]">
                            <div>{t('teamAdmin.externalUsers.detailPhone', '手机号')}: {selectedSummary.phone || '—'}</div>
                            <div>{t('teamAdmin.externalUsers.detailCreated', '创建时间')}: {formatDateTime(selectedSummary.createdAt)}</div>
                            <div>{t('teamAdmin.externalUsers.detailLastLogin', '最近登录')}: {selectedSummary.lastLoginAt ? formatDateTime(selectedSummary.lastLoginAt) : '—'}</div>
                            <div>{t('teamAdmin.externalUsers.detailLastSeen', '最近活跃')}: {selectedSummary.lastSeenAt ? formatDateTime(selectedSummary.lastSeenAt) : '—'}</div>
                          </div>
                        </div>

                        <div className="space-y-2">
                          <h4 className="text-sm font-semibold">{t('teamAdmin.externalUsers.linkedVisitors', '关联访客标识')}</h4>
                          {detail.linkedVisitorIds.length === 0 ? (
                            <p className="text-sm text-[hsl(var(--muted-foreground))]">—</p>
                          ) : (
                            <div className="flex flex-wrap gap-2">
                              {detail.linkedVisitorIds.map((visitorId) => (
                                <Badge key={visitorId} variant="outline">{visitorId}</Badge>
                              ))}
                            </div>
                          )}
                        </div>

                        <div className="space-y-2">
                          <h4 className="text-sm font-semibold">{t('teamAdmin.externalUsers.recentUploads', '最近上传')}</h4>
                          {detail.recentUploads.length === 0 ? (
                            <p className="text-sm text-[hsl(var(--muted-foreground))]">
                              {t('teamAdmin.externalUsers.noUploads', '暂无上传记录')}
                            </p>
                          ) : (
                            <div className="space-y-2">
                              {detail.recentUploads.map((doc) => (
                                <div key={doc.id} className="rounded-md border border-[hsl(var(--border))] px-3 py-2">
                                  <div className="font-medium">{doc.display_name || doc.name}</div>
                                  <div className="text-xs text-[hsl(var(--muted-foreground))]">
                                    {doc.mime_type} · {formatDateTime(doc.updated_at || doc.created_at)}
                                  </div>
                                </div>
                              ))}
                            </div>
                          )}
                        </div>

                        <div className="space-y-2">
                          <h4 className="text-sm font-semibold">{t('teamAdmin.externalUsers.recentSessions', '最近会话')}</h4>
                          {detail.recentSessions.length === 0 ? (
                            <p className="text-sm text-[hsl(var(--muted-foreground))]">
                              {t('teamAdmin.externalUsers.noSessions', '暂无会话记录')}
                            </p>
                          ) : (
                            <div className="space-y-2">
                              {detail.recentSessions.map((session) => (
                                <div key={session.sessionId} className="rounded-md border border-[hsl(var(--border))] px-3 py-2">
                                  <div className="flex items-center justify-between gap-3">
                                    <div className="font-medium">{session.title || session.portalSlug || session.sessionId}</div>
                                    {session.isProcessing && (
                                      <Badge variant="outline">{t('teamAdmin.externalUsers.processing', '处理中')}</Badge>
                                    )}
                                  </div>
                                  <div className="text-xs text-[hsl(var(--muted-foreground))]">
                                    {(session.portalSlug || 'portal') + ' · ' + formatDateTime(session.updatedAt) + ' · ' + t('teamAdmin.externalUsers.messageCount', '{{count}} 条消息', { count: session.messageCount })}
                                  </div>
                                </div>
                              ))}
                            </div>
                          )}
                        </div>
                      </div>
                    )}
                  </CardContent>
                </Card>

                <Card>
                  <CardHeader className="pb-2">
                    <CardTitle className="text-base">
                      {t('teamAdmin.externalUsers.eventsTitle', '最近事件')}
                    </CardTitle>
                    <CardDescription>
                      {t('teamAdmin.externalUsers.eventsDesc', '注册、登录、访客绑定和其他关键动作会记录在这里。')}
                    </CardDescription>
                  </CardHeader>
                  <CardContent>
                    {eventsLoading ? (
                      <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                        {t('common.loading')}
                      </p>
                    ) : events.length === 0 ? (
                      <p className="py-8 text-center text-sm text-[hsl(var(--muted-foreground))]">
                        {t('teamAdmin.externalUsers.noEvents', '暂无事件记录')}
                      </p>
                    ) : (
                      <div className="space-y-3">
                        {events.map((event) => (
                          <div key={event.id} className="rounded-md border border-[hsl(var(--border))] px-3 py-2">
                            <div className="flex items-center justify-between gap-3">
                              <div className="font-medium">{event.eventType}</div>
                              <Badge variant={event.result === 'success' ? 'default' : 'secondary'}>
                                {event.result}
                              </Badge>
                            </div>
                            <div className="mt-1 text-xs text-[hsl(var(--muted-foreground))]">
                              {formatDateTime(event.createdAt)}
                              {event.portalSlug ? ` · ${event.portalSlug}` : ''}
                              {event.visitorId ? ` · visitor ${event.visitorId}` : ''}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </CardContent>
                </Card>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>
    </>
  );

  if (isConversationTaskMode) {
    return (
      <>
        <MobileWorkspaceShell
          summary={(
            <div className="rounded-[20px] border border-border/60 bg-card px-4 py-3 shadow-[0_10px_22px_hsl(var(--ui-shadow))/0.03]">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                    {t('teamNav.externalUsers', '外部用户')}
                  </div>
                  <div className="mt-1 text-[15px] font-semibold tracking-[-0.02em] text-foreground">
                    {selectedSummary?.displayName || selectedSummary?.username || t('teamAdmin.externalUsers.title', '外部用户')}
                  </div>
                  <div className="mt-1 text-[11px] text-muted-foreground">
                    {selectedSummary
                      ? `${statusLabel(selectedSummary.status)} · ${t('teamAdmin.externalUsers.summaryEvents', '最近事件')} ${events.length}`
                      : t(
                          'teamAdmin.externalUsers.mobileConversationDescription',
                          '先处理用户与状态，再通过事件和上传线索判断是否需要进一步协作。',
                        )}
                  </div>
                </div>
                <Badge variant="outline" className="rounded-full px-2.5 py-0.5 text-[10px]">
                  {t('teamAdmin.externalUsers.summaryTotal', '用户总数')} {totalUsers}
                </Badge>
              </div>
            </div>
          )}
          quickActions={(
            <div className="grid grid-cols-3 gap-2">
              <Button variant="outline" className="h-10 justify-center rounded-[15px] border-border/60 px-2 text-[11px]" onClick={focusMobileSearch}>
                <Search className="mr-1.5 h-4 w-4" />
                {t('teamAdmin.externalUsers.quickSearchUsers', '搜索用户')}
              </Button>
              <Button
                variant="outline"
                className="h-10 justify-center rounded-[15px] border-border/60 px-2 text-[11px]"
                onClick={() => {
                  setMobileFilterSheetOpen(true);
                  setActiveMobilePanel('filters');
                }}
              >
                <SlidersHorizontal className="mr-1.5 h-4 w-4" />
                {t('teamAdmin.externalUsers.quickFilters', '筛选')}
              </Button>
              <Button
                variant="outline"
                className="h-10 justify-center rounded-[15px] border-border/60 px-2 text-[11px]"
                onClick={() => navigate(`/teams/${teamId}?section=chat`)}
              >
                <MessageSquareText className="mr-1.5 h-4 w-4" />
                {t('teamAdmin.externalUsers.quickChat', '对话协助')}
              </Button>
            </div>
          )}
          stage={mobileView === 'detail' ? renderMobileDetailView(true) : renderMobileListView(true)}
        >
          <ManagementRail
            title={t('teamAdmin.externalUsers.mobileRailTitle', '当前用户上下文')}
            description={t(
              'teamAdmin.externalUsers.mobileConversationRail',
              '只保留当前用户的关键状态与处理入口，详细资料退到详情页或面板。',
            )}
          >
            <div className="space-y-2.5">
              <div className="rounded-[16px] border border-border/60 bg-background px-3 py-2.5">
                <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground">
                  {t('teamAdmin.externalUsers.currentUser', '当前用户')}
                </div>
                <div className="mt-1 text-[13px] font-semibold text-foreground">
                  {selectedSummary?.displayName || selectedSummary?.username || '—'}
                </div>
                <div className="mt-1 text-[11px] text-muted-foreground">{mobileSummaryLine}</div>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <div className="rounded-[16px] border border-border/60 bg-background px-3 py-2.5">
                  <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground">
                    {t('teamAdmin.externalUsers.linkedVisitors', '关联访客')}
                  </div>
                  <div className="mt-1 text-[13px] font-semibold text-foreground">
                    {selectedSummary?.linkedVisitorCount ?? 0}
                  </div>
                </div>
                <div className="rounded-[16px] border border-border/60 bg-background px-3 py-2.5">
                  <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground">
                    {t('teamAdmin.externalUsers.summaryEvents', '最近事件')}
                  </div>
                  <div className="mt-1 text-[13px] font-semibold text-foreground">
                    {events.length}
                  </div>
                </div>
              </div>
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  className="h-9 flex-1 rounded-[14px] border-border/60"
                  disabled={!selectedSummary}
                  onClick={() => setMobileView('detail')}
                >
                  {t('common.view', '查看')}
                </Button>
                <Button
                  variant="outline"
                  className="h-9 flex-1 rounded-[14px] border-border/60"
                  disabled={!selectedSummary}
                  onClick={() => setActiveMobilePanel('events')}
                >
                  {t('teamAdmin.externalUsers.eventsTitle', '最近事件')}
                </Button>
              </div>
            </div>
          </ManagementRail>
        </MobileWorkspaceShell>
        {mobileFilterPanel}
        {mobileEventsPanel}
        {mobileResetPasswordPanel}
      </>
    );
  }

  if (isMobileLayout) {
    return (
      <>
        {mobileView === 'detail' ? renderMobileDetailView(false) : renderMobileListView(false)}
        {mobileFilterPanel}
        {mobileEventsPanel}
        {mobileResetPasswordPanel}
      </>
    );
  }

  return (
    <>
      {externalUsersContent}
      {desktopResetPasswordDialog}
    </>
  );
}
