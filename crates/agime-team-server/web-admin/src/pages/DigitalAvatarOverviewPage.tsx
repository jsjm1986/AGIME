import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowLeft, ExternalLink, RefreshCw, ShieldAlert } from 'lucide-react';
import { AppShell } from '../components/layout/AppShell';
import { TeamProvider } from '../contexts/TeamContext';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Badge } from '../components/ui/badge';
import { Skeleton } from '../components/ui/skeleton';
import { Input } from '../components/ui/input';
import { StatusBadge, PORTAL_STATUS_MAP } from '../components/ui/status-badge';
import { AgentTypeBadge, resolveAgentVisualType } from '../components/agent/AgentTypeBadge';
import { apiClient } from '../api/client';
import type { TeamWithStats } from '../api/types';
import {
  avatarPortalApi,
  type AvatarGovernanceQueueItemPayload,
  type AvatarInstanceProjection,
  type PortalSummary,
} from '../api/avatarPortal';
import { agentApi, type TeamAgent } from '../api/agent';
import { createEmptyGovernanceState, readGovernanceState } from '../components/team/digital-avatar/governance';
import { detectAvatarType } from '../components/team/digital-avatar/avatarType';
import { AvatarTypeBadge } from '../components/team/digital-avatar/AvatarTypeBadge';
import { formatRelativeTime } from '../utils/format';

type AvatarTypeFilter = 'all' | 'external' | 'internal';
type AttentionFilter = 'all' | 'human' | 'high' | 'pending';

interface AvatarOverviewRow {
  avatar: PortalSummary;
  projection: AvatarInstanceProjection | null;
  managerAgent: TeamAgent | null;
  serviceAgent: TeamAgent | null;
  queueItems: AvatarGovernanceQueueItemPayload[];
  pendingCount: number;
  needsHumanCount: number;
  highRiskCount: number;
  runtimePendingCount: number;
  latestActivity: string;
}

interface AggregatedGovernanceItem {
  avatarId: string;
  avatarName: string;
  avatarSlug: string;
  managerAgentName: string;
  kind: AvatarGovernanceQueueItemPayload['kind'];
  title: string;
  detail: string;
  status: string;
  ts: string;
  meta: string[];
  risk: 'low' | 'medium' | 'high';
  needsHuman: boolean;
}

function queueRisk(meta: string[]): 'low' | 'medium' | 'high' {
  const joined = meta.join(' ').toLowerCase();
  if (
    joined.includes('high')
    || joined.includes('高风险')
    || joined.includes('critical')
    || joined.includes('严重')
  ) {
    return 'high';
  }
  if (
    joined.includes('medium')
    || joined.includes('中风险')
    || joined.includes('moderate')
  ) {
    return 'medium';
  }
  return 'low';
}

function isHumanAttentionItem(item: AvatarGovernanceQueueItemPayload): boolean {
  if (item.kind === 'capability') return item.status === 'needs_human';
  return ['pending', 'pending_approval', 'pilot', 'approved', 'experimenting'].includes(item.status);
}

export default function DigitalAvatarOverviewPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { teamId } = useParams<{ teamId: string }>();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [rows, setRows] = useState<AvatarOverviewRow[]>([]);
  const [managerFilter, setManagerFilter] = useState('all');
  const [typeFilter, setTypeFilter] = useState<AvatarTypeFilter>('all');
  const [attentionFilter, setAttentionFilter] = useState<AttentionFilter>('all');
  const [search, setSearch] = useState('');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try {
      return window.localStorage.getItem('sidebar-collapsed') === 'true';
    } catch {
      return false;
    }
  });

  const canManage = Boolean(team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin');

  const handleSectionChange = useCallback((section: string) => {
    if (!teamId) return;
    navigate(`/teams/${teamId}?section=${section}`);
  }, [navigate, teamId]);

  const handleToggleSidebar = useCallback(() => {
    setSidebarCollapsed((prev) => {
      try {
        window.localStorage.setItem('sidebar-collapsed', String(!prev));
      } catch {
        // ignore storage failure
      }
      return !prev;
    });
  }, []);

  const focusAvatarInWorkspace = useCallback((avatarId: string) => {
    if (!teamId) return;
    try {
      window.localStorage.setItem(`digital_avatar_focus:v1:${teamId}`, avatarId);
    } catch {
      // ignore storage failure
    }
    navigate(`/teams/${teamId}?section=digital-avatar`);
  }, [navigate, teamId]);

  const loadData = useCallback(async () => {
    if (!teamId) return;
    try {
      setLoading(true);
      const [teamResult, portalResult, projectionResult, agentResult] = await Promise.all([
        apiClient.getTeam(teamId),
        avatarPortalApi.list(teamId, 1, 200),
        avatarPortalApi.listInstances(teamId).catch(() => [] as AvatarInstanceProjection[]),
        agentApi.listAgents(teamId, 1, 400).catch(() => ({ items: [] as TeamAgent[], total: 0, page: 1, limit: 400, totalPages: 1 })),
      ]);
      const avatars = portalResult.items || [];
      const projections = new Map(projectionResult.map((item) => [item.portalId, item]));
      const agents = agentResult.items || [];
      const governanceResults = await Promise.all(
        avatars.map(async (avatar) => {
          const [queue, governancePayload] = await Promise.all([
            avatarPortalApi.listGovernanceQueue(teamId, avatar.id).catch(() => [] as AvatarGovernanceQueueItemPayload[]),
            avatarPortalApi.getGovernance(teamId, avatar.id).catch(() => null),
          ]);
          const governance = governancePayload
            ? readGovernanceState({ digitalAvatarGovernance: governancePayload.state })
            : createEmptyGovernanceState();
          return [avatar.id, queue, governance] as const;
        }),
      );
      const nextRows: AvatarOverviewRow[] = avatars.map((avatar) => {
        const found = governanceResults.find((item) => item[0] === avatar.id);
        const queue = found?.[1] || [];
        const governance = found?.[2] || createEmptyGovernanceState();
        const projection = projections.get(avatar.id) || null;
        return {
          avatar,
          projection,
          managerAgent: agents.find((agent) => agent.id === (avatar.codingAgentId || avatar.agentId || null)) || null,
          serviceAgent: agents.find((agent) => agent.id === (avatar.serviceAgentId || avatar.agentId || null)) || null,
          queueItems: queue,
          pendingCount: queue.length,
          needsHumanCount: queue.filter(isHumanAttentionItem).length,
          highRiskCount: queue.filter((item) => queueRisk(item.meta) === 'high').length
            + governance.runtimeLogs.filter((item) => item.status === 'pending' && item.risk === 'high').length,
          runtimePendingCount: governance.runtimeLogs.filter((item) => item.status === 'pending').length,
          latestActivity: projection?.portalUpdatedAt || avatar.updatedAt || avatar.createdAt,
        };
      });
      setTeam(teamResult.team);
      setRows(nextRows);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [t, teamId]);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  const managerOptions = useMemo(() => {
    const map = new Map<string, string>();
    rows.forEach((row) => {
      if (row.managerAgent?.id) {
        map.set(row.managerAgent.id, row.managerAgent.name);
      }
    });
    return Array.from(map.entries()).map(([id, name]) => ({ id, name }));
  }, [rows]);

  const visibleRows = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    return rows.filter((row) => {
      if (managerFilter !== 'all' && row.managerAgent?.id !== managerFilter) return false;
      const avatarType = detectAvatarType(row.avatar, row.projection);
      if (typeFilter !== 'all' && avatarType !== typeFilter) return false;
      if (attentionFilter === 'human' && row.needsHumanCount === 0) return false;
      if (attentionFilter === 'high' && row.highRiskCount === 0) return false;
      if (attentionFilter === 'pending' && row.pendingCount === 0) return false;
      if (!keyword) return true;
      const haystack = [
        row.avatar.name,
        row.avatar.slug,
        row.avatar.description || '',
        row.managerAgent?.name || '',
        row.serviceAgent?.name || '',
      ].join(' ').toLowerCase();
      return haystack.includes(keyword);
    });
  }, [attentionFilter, managerFilter, rows, search, typeFilter]);

  const summary = useMemo(() => {
    const total = rows.length;
    const published = rows.filter((row) => row.avatar.status === 'published').length;
    const draftOrPaused = rows.filter((row) => row.avatar.status !== 'published').length;
    const pending = rows.reduce((sum, row) => sum + row.pendingCount, 0);
    const needsHuman = rows.reduce((sum, row) => sum + row.needsHumanCount, 0);
    const highRisk = rows.reduce((sum, row) => sum + row.highRiskCount, 0);
    return { total, published, draftOrPaused, pending, needsHuman, highRisk };
  }, [rows]);

  const aggregatedQueue = useMemo<AggregatedGovernanceItem[]>(() => {
    return visibleRows
      .flatMap((row) =>
        row.queueItems.map((item) => ({
          avatarId: row.avatar.id,
          avatarName: row.avatar.name,
          avatarSlug: row.avatar.slug,
          managerAgentName: row.managerAgent?.name || t('digitalAvatar.labels.unset', '未设置'),
          kind: item.kind,
          title: item.title,
          detail: item.detail,
          status: item.status,
          ts: item.ts,
          meta: item.meta,
          risk: queueRisk(item.meta),
          needsHuman: isHumanAttentionItem(item),
        })),
      )
      .sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts));
  }, [t, visibleRows]);

  const exportOverviewReport = useCallback(() => {
    if (!teamId) return;
    const lines: string[] = [];
    lines.push(`# ${t('digitalAvatar.overview.title', '数字分身治理总览')}`);
    lines.push('');
    lines.push(`- ${t('common.team', '团队')}: ${team?.name || teamId}`);
    lines.push(`- ${t('common.generatedAt', { defaultValue: '导出时间' })}: ${new Date().toLocaleString()}`);
    lines.push(`- ${t('digitalAvatar.overview.totalAvatars', '全部分身')}: ${summary.total}`);
    lines.push(`- ${t('digitalAvatar.overview.publishedAvatars', '已发布')}: ${summary.published}`);
    lines.push(`- ${t('digitalAvatar.overview.draftAvatars', '草稿 / 停用')}: ${summary.draftOrPaused}`);
    lines.push(`- ${t('digitalAvatar.overview.pendingGovernance', '待处理治理事项')}: ${summary.pending}`);
    lines.push(`- ${t('digitalAvatar.overview.needsHuman', '需人工确认')}: ${summary.needsHuman}`);
    lines.push(`- ${t('digitalAvatar.overview.highRisk', '高风险事项')}: ${summary.highRisk}`);
    lines.push('');
    lines.push('## 当前筛选');
    lines.push('');
    lines.push(`- ${t('digitalAvatar.overview.allManagers', '全部管理 Agent')}: ${managerFilter === 'all' ? t('common.all', '全部') : managerOptions.find((item) => item.id === managerFilter)?.name || managerFilter}`);
    lines.push(`- ${t('digitalAvatar.overview.typeFilter', { defaultValue: '分身类型' })}: ${typeFilter}`);
    lines.push(`- ${t('digitalAvatar.overview.attentionFilter', { defaultValue: '治理关注点' })}: ${attentionFilter}`);
    lines.push(`- ${t('common.search', '搜索')}: ${search.trim() || t('common.none', { defaultValue: '无' })}`);
    lines.push('');
    lines.push('## 分身清单');
    lines.push('');
    if (visibleRows.length === 0) {
      lines.push(`- ${t('digitalAvatar.overview.empty', '当前筛选条件下没有数字分身。')}`);
    } else {
      visibleRows.forEach((row) => {
        lines.push(`### ${row.avatar.name}`);
        lines.push(`- slug: ${row.avatar.slug}`);
        lines.push(`- status: ${row.avatar.status}`);
        lines.push(`- type: ${detectAvatarType(row.avatar, row.projection)}`);
        lines.push(`- manager: ${row.managerAgent?.name || t('digitalAvatar.labels.unset', '未设置')}`);
        lines.push(`- service: ${row.serviceAgent?.name || t('digitalAvatar.labels.unset', '未设置')}`);
        lines.push(`- pending: ${row.pendingCount}`);
        lines.push(`- needs_human: ${row.needsHumanCount}`);
        lines.push(`- high_risk: ${row.highRiskCount}`);
        lines.push(`- runtime_pending: ${row.runtimePendingCount}`);
        lines.push(`- latest_activity: ${formatRelativeTime(row.latestActivity)}`);
        lines.push('');
      });
    }

    const blob = new Blob([lines.join('\n')], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `digital-avatar-overview-${teamId}-${Date.now()}.md`;
    anchor.click();
    URL.revokeObjectURL(url);
  }, [
    attentionFilter,
    managerFilter,
    managerOptions,
    search,
    summary,
    t,
    team?.name,
    teamId,
    typeFilter,
    visibleRows,
  ]);

  if (loading) {
    return (
      <AppShell className="team-font-cap">
        <div className="space-y-4">
          <Skeleton className="h-12 w-72" />
          <Skeleton className="h-36 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      </AppShell>
    );
  }

  if (!team || error) {
    return (
      <AppShell className="team-font-cap">
        <div className="flex flex-col items-center justify-center gap-4 py-16 text-center">
          <p className="text-[hsl(var(--destructive))]">{error || t('teams.notFound')}</p>
          <Link to={teamId ? `/teams/${teamId}?section=digital-avatar` : '/teams'}>
            <Button variant="outline">{t('teams.backToList')}</Button>
          </Link>
        </div>
      </AppShell>
    );
  }

  return (
    <TeamProvider
      value={{
        team,
        canManage,
        activeSection: 'digital-avatar',
        onSectionChange: handleSectionChange,
        onInviteClick: () => undefined,
        sidebarCollapsed,
        onToggleSidebar: handleToggleSidebar,
      }}
    >
      <AppShell className="team-font-cap">
        <div className="space-y-6">
          <div className="flex items-center justify-between gap-3">
            <Button variant="ghost" size="sm" className="px-2" onClick={() => navigate(`/teams/${teamId}?section=digital-avatar`)}>
              <ArrowLeft className="mr-1.5 h-4 w-4" />
              {t('digitalAvatar.overview.backToWorkspace', '返回数字分身工作台')}
            </Button>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" onClick={exportOverviewReport}>
                {t('digitalAvatar.overview.export', '导出治理摘要')}
              </Button>
              <Button variant="outline" size="sm" onClick={() => navigate(`/teams/${teamId}/digital-avatars/audit`)}>
                <Activity className="mr-1.5 h-4 w-4" />
                {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
              </Button>
              <Button variant="outline" size="sm" onClick={() => navigate(`/teams/${teamId}/digital-avatars/policies`)}>
                <ShieldAlert className="mr-1.5 h-4 w-4" />
                {t('digitalAvatar.actions.policyCenter', '风险策略')}
              </Button>
              <Button variant="outline" size="sm" onClick={() => void loadData()}>
                <RefreshCw className="mr-1.5 h-4 w-4" />
                {t('common.refresh', '刷新')}
              </Button>
            </div>
          </div>

          <Card className="border-border/70">
            <CardHeader className="pb-3">
              <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
                <div className="space-y-2">
                  <CardTitle className="text-xl">{t('digitalAvatar.overview.title', '数字分身治理总览')}</CardTitle>
                  <p className="text-sm text-muted-foreground">
                    {t('digitalAvatar.overview.description', '从团队视角查看全部数字分身的状态、风险、待处理治理事项和最近活动。')}
                  </p>
                </div>
                <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground xl:w-[min(32vw,340px)] xl:max-w-[340px]">
                  <div><div>{t('digitalAvatar.overview.managerGroups', '管理 Agent 组')}</div><div className="mt-1 font-medium text-foreground">{managerOptions.length}</div></div>
                  <div><div>{t('digitalAvatar.overview.filteredAvatars', '当前筛选结果')}</div><div className="mt-1 font-medium text-foreground">{visibleRows.length}</div></div>
                  <div>
                    <div><AvatarTypeBadge type="external" /></div>
                    <div className="mt-1 font-medium text-foreground">{rows.filter((row) => detectAvatarType(row.avatar, row.projection) === 'external').length}</div>
                  </div>
                  <div>
                    <div><AvatarTypeBadge type="internal" /></div>
                    <div className="mt-1 font-medium text-foreground">{rows.filter((row) => detectAvatarType(row.avatar, row.projection) === 'internal').length}</div>
                  </div>
                </div>
              </div>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-3 xl:grid-cols-6">
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.overview.totalAvatars', '全部分身')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{summary.total}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.overview.publishedAvatars', '已发布')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{summary.published}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.overview.draftAvatars', '草稿 / 停用')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{summary.draftOrPaused}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.overview.pendingGovernance', '待处理治理事项')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{summary.pending}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.overview.needsHuman', '需人工确认')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{summary.needsHuman}</div></div>
              <div className="rounded-xl border border-status-error/25 bg-status-error/10 px-4 py-4"><div className="text-xs text-status-error-text">{t('digitalAvatar.overview.highRisk', '高风险事项')}</div><div className="mt-2 text-2xl font-semibold text-status-error-text">{summary.highRisk}</div></div>
            </CardContent>
          </Card>

          <Card className="border-border/70">
            <CardHeader className="pb-2">
              <CardTitle className="text-base">{t('digitalAvatar.overview.filterTitle', '筛选与定位')}</CardTitle>
            </CardHeader>
            <CardContent className="flex flex-wrap items-center gap-2">
              <select className="h-8 rounded-md border bg-background px-2 text-xs" value={managerFilter} onChange={(event) => setManagerFilter(event.target.value)}>
                <option value="all">{t('digitalAvatar.overview.allManagers', '全部管理 Agent')}</option>
                {managerOptions.map((manager) => (
                  <option key={manager.id} value={manager.id}>{manager.name}</option>
                ))}
              </select>
              <div className="flex flex-wrap gap-1">
                {(['all', 'external', 'internal'] as AvatarTypeFilter[]).map((value) => (
                  <button
                    key={value}
                    type="button"
                    className={`rounded border px-2 py-1 text-[11px] ${typeFilter === value ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`}
                    onClick={() => setTypeFilter(value)}
                  >
                    {value === 'all'
                      ? t('digitalAvatar.filters.all')
                      : value === 'external'
                      ? t('digitalAvatar.filters.external')
                      : t('digitalAvatar.filters.internal')}
                  </button>
                ))}
              </div>
              <div className="flex flex-wrap gap-1">
                {(['all', 'pending', 'human', 'high'] as AttentionFilter[]).map((value) => (
                  <button
                    key={value}
                    type="button"
                    className={`rounded border px-2 py-1 text-[11px] ${attentionFilter === value ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`}
                    onClick={() => setAttentionFilter(value)}
                  >
                    {value === 'all'
                      ? t('digitalAvatar.timeline.filterAll', '全部')
                      : value === 'pending'
                      ? t('digitalAvatar.overview.pendingOnly', '仅待处理')
                      : value === 'human'
                      ? t('digitalAvatar.overview.humanOnly', '仅人工审批')
                      : t('digitalAvatar.overview.highRiskOnly', '仅高风险')}
                  </button>
                ))}
              </div>
                <Input
                  className="h-8 w-full flex-1 text-xs sm:w-[min(24rem,100%)]"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder={t('digitalAvatar.overview.search', '搜索分身名称、slug、管理 Agent 或服务 Agent')}
              />
            </CardContent>
          </Card>

          <Card className="border-border/70">
            <CardHeader className="pb-2">
              <CardTitle className="text-base">{t('digitalAvatar.overview.queueTitle', '跨分身待处理治理队列')}</CardTitle>
              <p className="text-xs text-muted-foreground">
                {t('digitalAvatar.overview.queueHint', '把当前筛选范围内所有分身的待处理治理事项汇总在一起，适合团队级排队和分发。')}
              </p>
            </CardHeader>
            <CardContent className="space-y-2">
              {aggregatedQueue.length === 0 ? (
                <div className="rounded-lg border border-dashed border-border/70 bg-muted/10 px-4 py-6 text-sm text-muted-foreground">
                  {t('digitalAvatar.overview.queueEmpty', '当前筛选范围内没有待处理治理事项。')}
                </div>
              ) : (
                aggregatedQueue.slice(0, 12).map((item) => (
                  <div key={`${item.avatarId}:${item.kind}:${item.title}:${item.ts}`} className="rounded-lg border border-border/70 bg-muted/10 px-4 py-3">
                    <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
                      <div className="min-w-0 space-y-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <Badge variant="outline">{item.avatarName}</Badge>
                          <Badge variant="outline">/p/{item.avatarSlug}</Badge>
                          <Badge variant="outline">{item.managerAgentName}</Badge>
                          <Badge className={item.risk === 'high' ? 'border-status-error/40 bg-status-error/10 text-status-error-text' : item.risk === 'medium' ? 'border-status-warning/40 bg-status-warning/10 text-status-warning-text' : 'border-status-success/40 bg-status-success/10 text-status-success-text'}>
                            {item.risk === 'high'
                              ? t('digitalAvatar.overview.highRisk', '高风险事项')
                              : item.risk === 'medium'
                              ? t('digitalAvatar.overview.mediumRisk', { defaultValue: '中风险' })
                              : t('digitalAvatar.overview.lowRisk', { defaultValue: '低风险' })}
                          </Badge>
                          {item.needsHuman && (
                            <Badge className="border-status-warning/40 bg-status-warning/10 text-status-warning-text">
                              {t('digitalAvatar.overview.humanOnly', '仅人工审批')}
                            </Badge>
                          )}
                        </div>
                        <div className="text-sm font-medium text-foreground">{item.title}</div>
                        <div className="text-xs text-muted-foreground">{item.detail}</div>
                        <div className="text-[11px] text-muted-foreground">
                          {formatRelativeTime(item.ts)} · {item.meta.join(' · ') || item.status}
                        </div>
                      </div>
                      <div className="flex flex-wrap items-center gap-2 xl:justify-end">
                        <Button size="sm" variant="outline" onClick={() => focusAvatarInWorkspace(item.avatarId)}>
                          {t('digitalAvatar.overview.openWorkspace', '打开工作台')}
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => navigate(`/teams/${teamId}/digital-avatars/${item.avatarId}/timeline`)}>
                          {t('digitalAvatar.timeline.openStandalone', '打开治理时间线')}
                        </Button>
                      </div>
                    </div>
                  </div>
                ))
              )}
              {aggregatedQueue.length > 12 && (
                <p className="text-xs text-muted-foreground">
                  {t('digitalAvatar.overview.queueMore', {
                    defaultValue: '当前共 {{count}} 条待处理事项，此处只展示最近 12 条。',
                    count: aggregatedQueue.length,
                  })}
                </p>
              )}
            </CardContent>
          </Card>

          <div className="space-y-3">
            {visibleRows.length === 0 ? (
              <Card className="border-border/70">
                <CardContent className="py-10 text-center text-sm text-muted-foreground">
                  {t('digitalAvatar.overview.empty', '当前筛选条件下没有数字分身。')}
                </CardContent>
              </Card>
            ) : visibleRows.map((row) => {
              const avatarType = detectAvatarType(row.avatar, row.projection);
              const previewUrl = row.avatar.previewUrl;
              const publicUrl = row.avatar.publicUrl;
              return (
                <Card key={row.avatar.id} className="border-border/70">
                  <CardContent className="py-4">
                    <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                      <div className="min-w-0 flex-1 space-y-3">
                        <div className="flex flex-wrap items-center gap-2">
                          <p className="text-base font-semibold text-foreground">{row.avatar.name}</p>
                          <Badge variant="outline" className="text-[11px]">/p/{row.avatar.slug}</Badge>
                          <StatusBadge status={PORTAL_STATUS_MAP[row.avatar.status] || 'neutral'}>
                            {t(`digitalAvatar.status.${row.avatar.status}`, row.avatar.status)}
                          </StatusBadge>
                          <AvatarTypeBadge type={avatarType} />
                        </div>
                        <p className="text-sm text-muted-foreground">
                          {row.avatar.description || t('digitalAvatar.overview.descriptionFallback', '当前分身尚未填写业务说明。')}
                        </p>
                        <div className="grid gap-3 text-xs text-muted-foreground md:grid-cols-4">
                          <div>
                            <div>{t('digitalAvatar.labels.managerAgent', '管理 Agent')}</div>
                            <div className="mt-1 flex flex-wrap items-center gap-1.5 font-medium text-foreground">
                              {row.managerAgent ? <AgentTypeBadge type={resolveAgentVisualType(row.managerAgent)} /> : null}
                              <span>{row.managerAgent?.name || '-'}</span>
                            </div>
                          </div>
                          <div>
                            <div>{t('digitalAvatar.labels.serviceAgent', '分身 Agent')}</div>
                            <div className="mt-1 flex flex-wrap items-center gap-1.5 font-medium text-foreground">
                              {row.serviceAgent ? <AgentTypeBadge type={resolveAgentVisualType(row.serviceAgent)} /> : null}
                              <span>{row.serviceAgent?.name || '-'}</span>
                            </div>
                          </div>
                          <div><div>{t('digitalAvatar.overview.latestActivity', '最近活动')}</div><div className="mt-1 font-medium text-foreground">{formatRelativeTime(row.latestActivity)}</div></div>
                          <div><div>{t('digitalAvatar.workspace.summaryAccess', '文档模式')}</div><div className="mt-1 font-medium text-foreground">{row.avatar.documentAccessMode}</div></div>
                        </div>
                        <div className="flex flex-wrap gap-2 text-[11px]">
                          <span className="rounded-full border border-border/60 bg-background px-2 py-1 text-muted-foreground">
                            {t('digitalAvatar.overview.pendingBadge', '待处理 {{count}}', { count: row.pendingCount })}
                          </span>
                          <span className="rounded-full border border-status-warning/35 bg-status-warning/10 px-2 py-1 text-status-warning-text">
                            {t('digitalAvatar.overview.humanBadge', '人工审批 {{count}}', { count: row.needsHumanCount })}
                          </span>
                          <span className="rounded-full border border-status-error/35 bg-status-error/10 px-2 py-1 text-status-error-text">
                            {t('digitalAvatar.overview.highRiskBadge', '高风险 {{count}}', { count: row.highRiskCount })}
                          </span>
                          <span className="rounded-full border border-border/60 bg-background px-2 py-1 text-muted-foreground">
                            {t('digitalAvatar.overview.runtimePendingBadge', '运行建议 {{count}}', { count: row.runtimePendingCount })}
                          </span>
                        </div>
                      </div>
                      <div className="flex flex-wrap items-center gap-2 xl:w-[min(360px,34vw)] xl:justify-end">
                        <Button size="sm" variant="outline" onClick={() => focusAvatarInWorkspace(row.avatar.id)}>
                          {t('digitalAvatar.overview.openWorkspace', '打开工作台')}
                        </Button>
                        <Button size="sm" variant="outline" onClick={() => navigate(`/teams/${teamId}/digital-avatars/${row.avatar.id}/timeline`)}>
                          {t('digitalAvatar.timeline.pageTitle', '治理时间线')}
                        </Button>
                        <Button
                          size="sm"
                          variant="outline"
                          disabled={!previewUrl}
                          onClick={() => previewUrl && window.open(previewUrl, '_blank', 'noopener,noreferrer')}
                        >
                          <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                          {t('digitalAvatar.workspace.openPreviewPage', '打开管理预览')}
                        </Button>
                        <Button
                          size="sm"
                          disabled={!publicUrl}
                          onClick={() => publicUrl && window.open(publicUrl, '_blank', 'noopener,noreferrer')}
                        >
                          <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                          {t('digitalAvatar.workspace.openPublicPage', '打开访客页')}
                        </Button>
                      </div>
                    </div>
                  </CardContent>
                </Card>
              );
            })}
          </div>
        </div>
      </AppShell>
    </TeamProvider>
  );
}
