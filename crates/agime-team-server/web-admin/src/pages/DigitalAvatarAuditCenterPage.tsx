import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowLeft, Download, ExternalLink, RefreshCw } from 'lucide-react';
import { AppShell } from '../components/layout/AppShell';
import { TeamProvider } from '../contexts/TeamContext';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Badge } from '../components/ui/badge';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../components/ui/dialog';
import { Skeleton } from '../components/ui/skeleton';
import { Input } from '../components/ui/input';
import { Pagination } from '../components/ui/pagination';
import { StatusBadge, PORTAL_STATUS_MAP } from '../components/ui/status-badge';
import { AgentTypeBadge, resolveAgentVisualType } from '../components/agent/AgentTypeBadge';
import { apiClient } from '../api/client';
import type { TeamWithStats } from '../api/types';
import {
  avatarPortalApi,
  type AvatarGovernanceEventPayload,
  type AvatarInstanceProjection,
  type PortalSummary,
} from '../api/avatarPortal';
import { agentApi, type TeamAgent } from '../api/agent';
import { detectAvatarType } from '../components/team/digital-avatar/avatarType';
import { AvatarTypeBadge } from '../components/team/digital-avatar/AvatarTypeBadge';
import { getAvatarPortalStatusText, getDigitalAvatarStatusText } from '../components/team/digital-avatar/displayText';
import { formatDateTime, formatRelativeTime } from '../utils/format';
import { useMediaQuery } from '../hooks/useMediaQuery';

type AuditEntityFilter = 'all' | 'runtime' | 'capability' | 'proposal' | 'ticket' | 'config';
type AuditRiskFilter = 'all' | 'low' | 'medium' | 'high';
const PAGE_SIZE = 20;

function eventRisk(meta: Record<string, unknown>): AuditRiskFilter {
  const text = JSON.stringify(meta).toLowerCase();
  if (text.includes('high') || text.includes('高风险') || text.includes('critical')) return 'high';
  if (text.includes('medium') || text.includes('中风险') || text.includes('moderate')) return 'medium';
  if (text.includes('low') || text.includes('低风险')) return 'low';
  return 'all';
}

function eventBadgeClass(event: AvatarGovernanceEventPayload): string {
  const status = `${event.status || ''}`.toLowerCase();
  if (status.includes('reject') || status.includes('fail') || status.includes('rollback')) {
    return 'border-status-error/35 bg-status-error/10 text-status-error-text';
  }
  if (status.includes('approve') || status.includes('active') || status.includes('deploy') || status.includes('publish')) {
    return 'border-status-success/35 bg-status-success/10 text-status-success-text';
  }
  if (status.includes('pending') || status.includes('pilot') || status.includes('human')) {
    return 'border-status-warning/35 bg-status-warning/10 text-status-warning-text';
  }
  return 'border-border/60 bg-muted/30 text-muted-foreground';
}

function getAuditEventDisplayText(
  t: ReturnType<typeof useTranslation>['t'],
  event: AvatarGovernanceEventPayload,
): string {
  if (event.status) return getDigitalAvatarStatusText(t, event.status);
  if (event.event_type) return getDigitalAvatarStatusText(t, event.event_type);
  return t('digitalAvatar.labels.unset', '未设置');
}

function downloadTextFile(filename: string, content: string, mime = 'text/markdown;charset=utf-8'): void {
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}

export default function DigitalAvatarAuditCenterPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { teamId } = useParams<{ teamId: string }>();
  const isMobileLayout = useMediaQuery('(max-width: 1023px)');
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try {
      return window.localStorage.getItem('sidebar-collapsed') === 'true';
    } catch {
      return false;
    }
  });
  const [avatars, setAvatars] = useState<PortalSummary[]>([]);
  const [projections, setProjections] = useState<AvatarInstanceProjection[]>([]);
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [events, setEvents] = useState<AvatarGovernanceEventPayload[]>([]);
  const [avatarFilter, setAvatarFilter] = useState('all');
  const [entityFilter, setEntityFilter] = useState<AuditEntityFilter>('all');
  const [riskFilter, setRiskFilter] = useState<AuditRiskFilter>('all');
  const [actorFilter, setActorFilter] = useState('all');
  const [search, setSearch] = useState('');
  const [page, setPage] = useState(1);
  const [filtersOpen, setFiltersOpen] = useState(false);

  const canManage = Boolean(team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin');

  const getEntityTypeLabel = useCallback((value: AuditEntityFilter | string) => {
    switch (value) {
      case 'runtime':
        return t('digitalAvatar.audit.entityType.runtime', { defaultValue: '运行' });
      case 'capability':
        return t('digitalAvatar.audit.entityType.capability', { defaultValue: '提权' });
      case 'proposal':
        return t('digitalAvatar.audit.entityType.proposal', { defaultValue: '新分身' });
      case 'ticket':
        return t('digitalAvatar.audit.entityType.ticket', { defaultValue: '优化' });
      case 'config':
        return t('digitalAvatar.audit.entityType.config', { defaultValue: '配置' });
      default:
        return value;
    }
  }, [t]);

  const getRiskLabel = useCallback((value: AuditRiskFilter | string) => {
    switch (value) {
      case 'low':
        return t('digitalAvatar.timeline.risk.low', { defaultValue: '低风险' });
      case 'medium':
        return t('digitalAvatar.timeline.risk.medium', { defaultValue: '中风险' });
      case 'high':
        return t('digitalAvatar.timeline.risk.high', { defaultValue: '高风险' });
      case 'all':
        return t('digitalAvatar.audit.allRisks', { defaultValue: '全部风险' });
      default:
        return value;
    }
  }, [t]);

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

  const loadData = useCallback(async () => {
    if (!teamId) return;
    try {
      setLoading(true);
      const [teamResult, avatarResult, projectionResult, eventResult, agentResult] = await Promise.all([
        apiClient.getTeam(teamId),
        avatarPortalApi.list(teamId, 1, 300),
        avatarPortalApi.listInstances(teamId).catch(() => [] as AvatarInstanceProjection[]),
        avatarPortalApi.listTeamGovernanceEvents(teamId, 500).catch(() => [] as AvatarGovernanceEventPayload[]),
        agentApi.listAgents(teamId, 1, 400).catch(() => ({ items: [] as TeamAgent[], total: 0, page: 1, limit: 400, totalPages: 1 })),
      ]);
      setTeam(teamResult.team);
      setAvatars(avatarResult.items || []);
      setProjections(projectionResult);
      setEvents(eventResult);
      setAgents(agentResult.items || []);
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

  const avatarMap = useMemo(() => new Map(avatars.map((avatar) => [avatar.id, avatar])), [avatars]);
  const projectionMap = useMemo(() => new Map(projections.map((item) => [item.portalId, item])), [projections]);
  const agentNameMap = useMemo(() => new Map(agents.map((agent) => [agent.id, agent.name])), [agents]);
  const agentMap = useMemo(() => new Map(agents.map((agent) => [agent.id, agent])), [agents]);

  const actorOptions = useMemo(() => {
    const actors = new Set<string>();
    events.forEach((event) => {
      const actor = (event.actor_name || '').trim();
      if (actor) actors.add(actor);
    });
    return Array.from(actors).sort((a, b) => a.localeCompare(b));
  }, [events]);

  const visibleEvents = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    return events.filter((event) => {
      if (avatarFilter !== 'all' && event.portal_id !== avatarFilter) return false;
      if (entityFilter !== 'all' && event.entity_type !== entityFilter) return false;
      if (riskFilter !== 'all' && eventRisk(event.meta) !== riskFilter) return false;
      if (actorFilter !== 'all' && (event.actor_name || '') !== actorFilter) return false;
      if (!keyword) return true;
      const avatar = avatarMap.get(event.portal_id);
      const haystack = [
        avatar?.name || '',
        avatar?.slug || '',
        event.title,
        event.detail || '',
        event.actor_name || '',
        JSON.stringify(event.meta),
      ].join(' ').toLowerCase();
      return haystack.includes(keyword);
    });
  }, [actorFilter, avatarFilter, avatarMap, entityFilter, events, riskFilter, search]);

  useEffect(() => {
    setPage(1);
  }, [avatarFilter, entityFilter, riskFilter, actorFilter, search]);

  useEffect(() => {
    if (!isMobileLayout) {
      setFiltersOpen(false);
    }
  }, [isMobileLayout]);

  const totalPages = Math.max(1, Math.ceil(visibleEvents.length / PAGE_SIZE));

  useEffect(() => {
    setPage((current) => Math.min(current, totalPages));
  }, [totalPages]);

  const pagedEvents = useMemo(() => {
    const start = (page - 1) * PAGE_SIZE;
    return visibleEvents.slice(start, start + PAGE_SIZE);
  }, [page, visibleEvents]);

  const summary = useMemo(() => {
    const highRisk = visibleEvents.filter((event) => eventRisk(event.meta) === 'high').length;
    const runtime = visibleEvents.filter((event) => event.entity_type === 'runtime').length;
    const config = visibleEvents.filter((event) => event.entity_type === 'config').length;
    const affectedAvatars = new Set(visibleEvents.map((event) => event.portal_id)).size;
    return {
      total: visibleEvents.length,
      highRisk,
      runtime,
      config,
      affectedAvatars,
    };
  }, [visibleEvents]);

  const exportAuditReport = useCallback(() => {
    if (!teamId) return;
    const lines: string[] = [];
    lines.push(`# ${t('digitalAvatar.audit.title', { defaultValue: '数字分身审计中心' })}`);
    lines.push('');
    lines.push(`- ${t('common.team', '团队')}: ${team?.name || teamId}`);
    lines.push(`- ${t('common.generatedAt', { defaultValue: '导出时间' })}: ${new Date().toLocaleString()}`);
    lines.push(`- ${t('digitalAvatar.audit.totalEvents', { defaultValue: '当前事件数' })}: ${summary.total}`);
    lines.push(`- ${t('digitalAvatar.audit.affectedAvatars', { defaultValue: '涉及分身' })}: ${summary.affectedAvatars}`);
    lines.push(`- ${t('digitalAvatar.overview.highRisk', '高风险事项')}: ${summary.highRisk}`);
    lines.push(`- ${t('digitalAvatar.audit.runtimeEvents', { defaultValue: '运行事件' })}: ${summary.runtime}`);
    lines.push(`- ${t('digitalAvatar.audit.configEvents', { defaultValue: '配置事件' })}: ${summary.config}`);
    lines.push('');
    lines.push('## Filters');
    lines.push(`- avatar: ${avatarFilter === 'all' ? t('common.all', '全部') : avatarMap.get(avatarFilter)?.name || avatarFilter}`);
    lines.push(`- entity: ${entityFilter === 'all' ? t('common.all', '全部') : getEntityTypeLabel(entityFilter)}`);
    lines.push(`- risk: ${getRiskLabel(riskFilter)}`);
    lines.push(`- actor: ${actorFilter === 'all' ? t('common.all', '全部') : actorFilter}`);
    lines.push(`- search: ${search.trim() || t('common.none', { defaultValue: '无' })}`);
    lines.push('');
    lines.push('## Events');
    lines.push('');
    if (visibleEvents.length === 0) {
      lines.push(`- ${t('digitalAvatar.audit.empty', { defaultValue: '当前筛选条件下没有治理事件。' })}`);
    } else {
      visibleEvents.forEach((event) => {
        const avatar = avatarMap.get(event.portal_id);
        lines.push(`### ${avatar?.name || event.portal_id} · ${event.title}`);
        lines.push(`- slug: ${avatar?.slug || 'n/a'}`);
        lines.push(`- entity: ${getEntityTypeLabel(event.entity_type)}`);
        lines.push(`- event_type: ${event.event_type}`);
        lines.push(`- status: ${event.status || 'n/a'}`);
        lines.push(`- risk: ${getRiskLabel(eventRisk(event.meta))}`);
        lines.push(`- actor: ${event.actor_name || 'system'}`);
        lines.push(`- created_at: ${event.created_at}`);
        if (event.detail) lines.push(`- detail: ${event.detail}`);
        lines.push('');
      });
    }

    downloadTextFile(`digital-avatar-audit-${teamId}-${Date.now()}.md`, lines.join('\n'));
  }, [actorFilter, avatarFilter, avatarMap, entityFilter, getEntityTypeLabel, getRiskLabel, riskFilter, search, summary, t, team?.name, teamId, visibleEvents]);

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
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <Button variant="ghost" size="sm" className="px-2" onClick={() => navigate(`/teams/${teamId}?section=digital-avatar`)}>
              <ArrowLeft className="mr-1.5 h-4 w-4" />
              {t('digitalAvatar.audit.backToWorkspace', { defaultValue: '返回数字分身工作台' })}
            </Button>
            <div className="flex w-full flex-wrap items-center gap-2 sm:w-auto sm:justify-end">
              <Button variant="outline" size="sm" className="flex-1 sm:flex-none" onClick={() => navigate(`/teams/${teamId}/digital-avatars/overview`)}>
                {t('digitalAvatar.actions.overview', '治理总览')}
              </Button>
              <Button variant="outline" size="sm" className="flex-1 sm:flex-none" onClick={() => navigate(`/teams/${teamId}/digital-avatars/policies`)}>
                {t('digitalAvatar.actions.policyCenter', '风险策略')}
              </Button>
              <Button variant="outline" size="sm" className="flex-1 sm:flex-none" onClick={exportAuditReport}>
                <Download className="mr-1.5 h-4 w-4" />
                {t('digitalAvatar.audit.export', { defaultValue: '导出审计摘要' })}
              </Button>
              <Button variant="outline" size="sm" className="flex-1 sm:flex-none" onClick={() => void loadData()}>
                <RefreshCw className="mr-1.5 h-4 w-4" />
                {t('common.refresh', '刷新')}
              </Button>
            </div>
          </div>

          <Card className="border-border/70">
            <CardHeader className="pb-3">
              <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
                <div className="space-y-2">
                  <CardTitle className="text-xl flex items-center gap-2">
                    <Activity className="h-5 w-5" />
                    {t('digitalAvatar.audit.title', { defaultValue: '数字分身审计中心' })}
                  </CardTitle>
                  <p className="text-sm text-muted-foreground">
                    {t('digitalAvatar.audit.description', { defaultValue: '从团队维度追踪全部分身的治理动作、配置变化和运行事件，适合复盘与运营审计。' })}
                  </p>
                </div>
                <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground xl:w-[min(34vw,360px)] xl:max-w-[360px]">
                  <div><div>{t('digitalAvatar.audit.totalEvents', { defaultValue: '当前事件数' })}</div><div className="mt-1 text-xl font-semibold text-foreground">{summary.total}</div></div>
                  <div><div>{t('digitalAvatar.audit.affectedAvatars', { defaultValue: '涉及分身' })}</div><div className="mt-1 text-xl font-semibold text-foreground">{summary.affectedAvatars}</div></div>
                  <div><div>{t('digitalAvatar.audit.runtimeEvents', { defaultValue: '运行事件' })}</div><div className="mt-1 text-xl font-semibold text-foreground">{summary.runtime}</div></div>
                  <div><div>{t('digitalAvatar.overview.highRisk', '高风险事项')}</div><div className="mt-1 text-xl font-semibold text-status-error-text">{summary.highRisk}</div></div>
                </div>
              </div>
            </CardHeader>
          </Card>

          {isMobileLayout ? (
            <Card className="border-border/70">
              <CardHeader className="pb-2">
                <CardTitle className="text-base">{t('digitalAvatar.audit.filters', { defaultValue: '筛选与检索' })}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                <Input
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder={t('digitalAvatar.audit.search', { defaultValue: '搜索分身、slug、事件标题、执行人或详细说明' })}
                  className="h-10 w-full text-sm"
                />
                <div className="flex items-center justify-between gap-3 rounded-xl border border-border/60 bg-muted/10 px-3 py-3 text-sm text-muted-foreground">
                  <div>
                    {t('digitalAvatar.audit.totalEvents', { defaultValue: '当前事件数' })}
                    <span className="ml-2 font-semibold text-foreground">{visibleEvents.length}</span>
                  </div>
                  <Button size="sm" variant="outline" onClick={() => setFiltersOpen(true)}>
                    {t('digitalAvatar.audit.openFilters', { defaultValue: '打开筛选' })}
                  </Button>
                </div>
              </CardContent>
            </Card>
          ) : (
            <Card className="border-border/70">
              <CardHeader className="pb-2">
                <CardTitle className="text-base">{t('digitalAvatar.audit.filters', { defaultValue: '筛选与检索' })}</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-wrap items-center gap-2">
                <select className="h-8 rounded-md border bg-background px-2 text-xs" value={avatarFilter} onChange={(event) => setAvatarFilter(event.target.value)}>
                  <option value="all">{t('digitalAvatar.audit.allAvatars', { defaultValue: '全部分身' })}</option>
                  {avatars.map((avatar) => (
                    <option key={avatar.id} value={avatar.id}>{avatar.name}</option>
                  ))}
                </select>
                <select className="h-8 rounded-md border bg-background px-2 text-xs" value={entityFilter} onChange={(event) => setEntityFilter(event.target.value as AuditEntityFilter)}>
                  <option value="all">{t('digitalAvatar.audit.allEventTypes', { defaultValue: '全部事件类型' })}</option>
                  {(['runtime', 'capability', 'proposal', 'ticket', 'config'] as AuditEntityFilter[]).map((value) => (
                    <option key={value} value={value}>{getEntityTypeLabel(value)}</option>
                  ))}
                </select>
                <select className="h-8 rounded-md border bg-background px-2 text-xs" value={riskFilter} onChange={(event) => setRiskFilter(event.target.value as AuditRiskFilter)}>
                  <option value="all">{t('digitalAvatar.audit.allRisks', { defaultValue: '全部风险' })}</option>
                  <option value="low">{getRiskLabel('low')}</option>
                  <option value="medium">{getRiskLabel('medium')}</option>
                  <option value="high">{getRiskLabel('high')}</option>
                </select>
                <select className="h-8 rounded-md border bg-background px-2 text-xs" value={actorFilter} onChange={(event) => setActorFilter(event.target.value)}>
                  <option value="all">{t('digitalAvatar.audit.allActors', { defaultValue: '全部执行人' })}</option>
                  {actorOptions.map((actor) => (
                    <option key={actor} value={actor}>{actor}</option>
                  ))}
                </select>
                <Input
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder={t('digitalAvatar.audit.search', { defaultValue: '搜索分身、slug、事件标题、执行人或详细说明' })}
                  className="w-full max-w-full sm:w-[min(26rem,100%)]"
                />
              </CardContent>
            </Card>
          )}

          <Card className="border-border/70">
            <CardHeader className="pb-2">
              <CardTitle className="text-base">{t('digitalAvatar.audit.timelineTitle', { defaultValue: '团队级治理时间线' })}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {visibleEvents.length === 0 ? (
                <div className="rounded-xl border border-dashed border-border/70 bg-muted/10 px-4 py-10 text-center text-sm text-muted-foreground">
                  {t('digitalAvatar.audit.empty', { defaultValue: '当前筛选条件下没有治理事件。' })}
                </div>
              ) : pagedEvents.map((event) => {
                const avatar = avatarMap.get(event.portal_id);
                const projection = projectionMap.get(event.portal_id);
                const managerName = projection?.managerAgentId ? (agentNameMap.get(projection.managerAgentId) || projection.managerAgentId) : t('digitalAvatar.labels.unset', '未设置');
                const managerAgent = projection?.managerAgentId ? (agentMap.get(projection.managerAgentId) || null) : null;
                const risk = eventRisk(event.meta);
                const avatarType = avatar
                  ? detectAvatarType(avatar, projectionMap.get(avatar.id))
                  : 'unknown';
                return (
                  <div key={event.event_id} className="rounded-xl border border-border/70 bg-background px-4 py-4">
                    <div className="space-y-3">
                      <div className="space-y-2">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-sm font-semibold text-foreground">{event.title}</span>
                          {avatar ? (
                            <StatusBadge status={PORTAL_STATUS_MAP[avatar.status] || 'neutral'} className="text-[11px]">
                              {getAvatarPortalStatusText(t, avatar.status)}
                            </StatusBadge>
                          ) : null}
                          <AvatarTypeBadge type={avatarType} />
                          <Badge variant="outline" className={eventBadgeClass(event)}>{getAuditEventDisplayText(t, event)}</Badge>
                          <Badge variant="outline" className="text-[11px]">{getEntityTypeLabel(event.entity_type)}</Badge>
                          {risk !== 'all' ? <Badge variant="outline" className={risk === 'high' ? 'border-status-error/35 bg-status-error/10 text-status-error-text' : risk === 'medium' ? 'border-status-warning/35 bg-status-warning/10 text-status-warning-text' : 'border-status-success/35 bg-status-success/10 text-status-success-text'}>{getRiskLabel(risk)}</Badge> : null}
                        </div>
                        <div className="text-xs text-muted-foreground">
                          {(avatar?.name || event.portal_id)} · /{avatar?.slug || projection?.slug || 'unknown'}
                        </div>
                        <div className="text-sm text-muted-foreground">{event.detail || t('digitalAvatar.audit.noDetail', { defaultValue: '该事件未附带详细说明。' })}</div>
                      </div>
                      <div className="grid gap-3 border-t border-border/55 pt-3 text-xs text-muted-foreground lg:grid-cols-[minmax(0,1fr)_auto] lg:items-end">
                        <div className="grid gap-x-6 gap-y-2 sm:grid-cols-2 xl:grid-cols-4">
                          <div className="space-y-1">
                            <div className="font-medium text-foreground">{formatDateTime(event.created_at)}</div>
                            <div>{t('digitalAvatar.audit.relativeTime', { defaultValue: '相对时间' })}: {formatRelativeTime(event.created_at)}</div>
                          </div>
                          <div className="space-y-1">
                            <div>{t('digitalAvatar.audit.actor', { defaultValue: '执行人' })}</div>
                            <div className="font-medium text-foreground">{event.actor_name || t('digitalAvatar.audit.actorSystem', { defaultValue: '系统' })}</div>
                          </div>
                          <div className="space-y-1 sm:col-span-2 xl:col-span-2">
                            <div>{t('digitalAvatar.audit.managerAgent', { defaultValue: '管理 Agent' })}</div>
                            <div className="flex flex-wrap items-center gap-1.5 font-medium text-foreground">
                              {managerAgent ? <AgentTypeBadge type={resolveAgentVisualType(managerAgent)} /> : null}
                              <span>{managerName}</span>
                            </div>
                          </div>
                        </div>
                        <div className="flex flex-col gap-2 sm:flex-row lg:justify-end">
                          <Button variant="outline" size="sm" className="w-full sm:w-auto" onClick={() => navigate(`/teams/${teamId}?section=digital-avatar`)}>
                            {t('digitalAvatar.overview.openWorkspace', '打开工作台')}
                          </Button>
                          <Button variant="outline" size="sm" className="w-full sm:w-auto" onClick={() => navigate(`/teams/${teamId}/digital-avatars/${event.portal_id}/timeline`)}>
                            <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                            {t('digitalAvatar.timeline.title', '治理时间线')}
                          </Button>
                        </div>
                      </div>
                    </div>
                  </div>
                );
              })}
              {visibleEvents.length > PAGE_SIZE && (
                <div className="pt-1">
                  <Pagination
                    currentPage={page}
                    totalPages={totalPages}
                    totalItems={visibleEvents.length}
                    pageSize={PAGE_SIZE}
                    onPageChange={setPage}
                  />
                </div>
              )}
            </CardContent>
          </Card>
        </div>
        <Dialog open={filtersOpen} onOpenChange={setFiltersOpen}>
          <DialogContent className="max-h-[88vh] overflow-hidden sm:max-w-lg">
            <DialogHeader>
              <DialogTitle>{t('digitalAvatar.audit.filters', { defaultValue: '筛选与检索' })}</DialogTitle>
            </DialogHeader>
            <div className="space-y-4 overflow-y-auto pr-1">
              <div className="space-y-2">
                <label className="text-xs font-medium text-muted-foreground">
                  {t('digitalAvatar.audit.allAvatars', { defaultValue: '全部分身' })}
                </label>
                <select className="h-10 w-full rounded-md border bg-background px-3 text-sm" value={avatarFilter} onChange={(event) => setAvatarFilter(event.target.value)}>
                  <option value="all">{t('digitalAvatar.audit.allAvatars', { defaultValue: '全部分身' })}</option>
                  {avatars.map((avatar) => (
                    <option key={avatar.id} value={avatar.id}>{avatar.name}</option>
                  ))}
                </select>
              </div>
              <div className="space-y-2">
                <label className="text-xs font-medium text-muted-foreground">
                  {t('digitalAvatar.audit.allEventTypes', { defaultValue: '全部事件类型' })}
                </label>
                <select className="h-10 w-full rounded-md border bg-background px-3 text-sm" value={entityFilter} onChange={(event) => setEntityFilter(event.target.value as AuditEntityFilter)}>
                  <option value="all">{t('digitalAvatar.audit.allEventTypes', { defaultValue: '全部事件类型' })}</option>
                  {(['runtime', 'capability', 'proposal', 'ticket', 'config'] as AuditEntityFilter[]).map((value) => (
                    <option key={value} value={value}>{getEntityTypeLabel(value)}</option>
                  ))}
                </select>
              </div>
              <div className="space-y-2">
                <label className="text-xs font-medium text-muted-foreground">
                  {t('digitalAvatar.audit.allRisks', { defaultValue: '全部风险' })}
                </label>
                <select className="h-10 w-full rounded-md border bg-background px-3 text-sm" value={riskFilter} onChange={(event) => setRiskFilter(event.target.value as AuditRiskFilter)}>
                  <option value="all">{t('digitalAvatar.audit.allRisks', { defaultValue: '全部风险' })}</option>
                  <option value="low">{getRiskLabel('low')}</option>
                  <option value="medium">{getRiskLabel('medium')}</option>
                  <option value="high">{getRiskLabel('high')}</option>
                </select>
              </div>
              <div className="space-y-2">
                <label className="text-xs font-medium text-muted-foreground">
                  {t('digitalAvatar.audit.allActors', { defaultValue: '全部执行人' })}
                </label>
                <select className="h-10 w-full rounded-md border bg-background px-3 text-sm" value={actorFilter} onChange={(event) => setActorFilter(event.target.value)}>
                  <option value="all">{t('digitalAvatar.audit.allActors', { defaultValue: '全部执行人' })}</option>
                  {actorOptions.map((actor) => (
                    <option key={actor} value={actor}>{actor}</option>
                  ))}
                </select>
              </div>
              <Button className="w-full" onClick={() => setFiltersOpen(false)}>
                {t('common.done', '完成')}
              </Button>
            </div>
          </DialogContent>
        </Dialog>
      </AppShell>
    </TeamProvider>
  );
}
