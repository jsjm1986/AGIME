import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
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
import { useToast } from '../../contexts/ToastContext';
import {
  externalUsersApi,
  type ExternalUserDetail,
  type ExternalUserEventResponse,
  type ExternalUserStatus,
  type ExternalUserSummary,
} from '../../api/externalUsers';
import { formatDateTime } from '../../utils/format';

interface ExternalUsersTabProps {
  teamId: string;
}

function statusVariant(status: ExternalUserStatus) {
  return status === 'active' ? 'default' as const : 'secondary' as const;
}

export function ExternalUsersTab({ teamId }: ExternalUsersTabProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();

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

  const selectedSummary = useMemo(
    () => users.find((user) => user.id === selectedUserId) ?? detail?.user ?? null,
    [detail?.user, selectedUserId, users],
  );

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

  const applySearch = () => {
    setPage(1);
    setSearchTerm(searchInput.trim());
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
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setResetSaving(false);
    }
  };

  return (
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

      <Dialog open={!!resetTarget} onOpenChange={(open) => {
        if (!open) {
          setResetTarget(null);
          setNewPassword('');
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
                setResetTarget(null);
                setNewPassword('');
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
    </>
  );
}
