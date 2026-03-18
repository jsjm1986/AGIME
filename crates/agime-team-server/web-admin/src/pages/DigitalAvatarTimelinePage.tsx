import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowLeft, Download, ExternalLink, Filter, Loader2, RefreshCw, ShieldAlert } from 'lucide-react';
import { AppShell } from '../components/layout/AppShell';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Badge } from '../components/ui/badge';
import { Skeleton } from '../components/ui/skeleton';
import { Input } from '../components/ui/input';
import { StatusBadge, AGENT_STATUS_MAP, PORTAL_STATUS_MAP } from '../components/ui/status-badge';
import { AgentTypeBadge, resolveAgentVisualType } from '../components/agent/AgentTypeBadge';
import { TeamProvider } from '../contexts/TeamContext';
import { useToast } from '../contexts/ToastContext';
import { apiClient } from '../api/client';
import type { TeamWithStats } from '../api/types';
import {
  avatarPortalApi,
  type AvatarGovernanceEventPayload,
  type AvatarGovernanceQueueItemPayload,
  type PortalDetail,
} from '../api/avatarPortal';
import { chatApi, type ChatSessionEvent } from '../api/chat';
import { agentApi, type TeamAgent } from '../api/agent';
import { formatDateTime, formatRelativeTime } from '../utils/format';
import {
  createEmptyGovernanceState,
  readGovernanceState,
  type AvatarGovernanceState,
  type DecisionMode,
  type OptimizationStatus,
  type ProposalStatus,
  type RuntimeLogStatus,
} from '../components/team/digital-avatar/governance';
import { detectAvatarType } from '../components/team/digital-avatar/avatarType';
import { AvatarTypeBadge } from '../components/team/digital-avatar/AvatarTypeBadge';
import {
  formatDigitalAvatarMetaLabel,
  getAvatarPortalStatusText,
  getDigitalAvatarDocumentAccessModeText,
} from '../components/team/digital-avatar/displayText';

type PersistedEventFilter = 'all' | 'error' | 'tool' | 'thinking' | 'status';
type PersistedEventLoadMode = 'latest' | 'older' | 'incremental';
type QueueKindFilter = 'all' | 'capability' | 'proposal' | 'ticket';
type ReviewStateFilter = 'all' | 'open' | 'resolved';
type GovernanceRowType = 'runtime' | 'capability' | 'proposal' | 'ticket';
type TimelineKindFilter = 'all' | GovernanceRowType;
type RiskFilter = 'all' | 'low' | 'medium' | 'high';

const STORAGE_KEY = 'sidebar-collapsed';
const PERSISTED_EVENTS_PAGE_SIZE = 200;
const MANAGER_COMPOSE_STORAGE_PREFIX = 'digital_avatar_manager_compose:v1:';
const MANAGER_FOCUS_STORAGE_PREFIX = 'digital_avatar_focus:v1:';

interface GovernanceTimelineRow {
  id: string;
  ts: string;
  rowType: GovernanceRowType;
  title: string;
  detail: string;
  status: string;
  meta: string[];
}

interface BatchGovernanceSummary {
  kind: 'capability' | 'proposal' | 'ticket';
  count: number;
  actionLabel: string;
  updatedAt: string;
}

function badgeClass(status: string): string {
  switch (status) {
    case 'approved':
    case 'active':
    case 'deployed':
    case 'published':
    case 'success':
      return 'border-status-success/35 bg-status-success/10 text-status-success-text';
    case 'needs_human':
    case 'pending':
    case 'pending_approval':
    case 'pilot':
    case 'experimenting':
    case 'draft':
      return 'border-status-warning/35 bg-status-warning/10 text-status-warning-text';
    case 'rejected':
    case 'rolled_back':
    case 'failed':
    case 'error':
      return 'border-status-error/35 bg-status-error/10 text-status-error-text';
    default:
      return 'border-border/60 bg-muted/30 text-muted-foreground';
  }
}

function runtimeStatusClass(status: RuntimeLogStatus): string {
  switch (status) {
    case 'ticketed':
    case 'requested':
      return 'border-status-info/35 bg-status-info/10 text-status-info-text';
    case 'dismissed':
      return 'border-border/60 bg-muted/30 text-muted-foreground';
    default:
      return 'border-status-warning/35 bg-status-warning/10 text-status-warning-text';
  }
}

function getGovernanceStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: string,
): string {
  switch (status) {
    case 'pending':
      return t('digitalAvatar.governance.status.pending', '待决策');
    case 'approved':
      return t('digitalAvatar.governance.status.approved', '已通过');
    case 'needs_human':
      return t('digitalAvatar.governance.status.needs_human', '需人工确认');
    case 'rejected':
      return t('digitalAvatar.governance.status.rejected', '已拒绝');
    default:
      return status;
  }
}

function getProposalStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: ProposalStatus | string,
): string {
  switch (status) {
    case 'draft':
      return t('digitalAvatar.governance.proposalStatus.draft', '草稿');
    case 'pending_approval':
      return t('digitalAvatar.governance.proposalStatus.pending_approval', '待审批');
    case 'approved':
      return t('digitalAvatar.governance.proposalStatus.approved', '已通过');
    case 'rejected':
      return t('digitalAvatar.governance.proposalStatus.rejected', '已拒绝');
    case 'pilot':
      return t('digitalAvatar.governance.proposalStatus.pilot', '试运行');
    case 'active':
      return t('digitalAvatar.governance.proposalStatus.active', '生效中');
    default:
      return status;
  }
}

function getOptimizationStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: OptimizationStatus | string,
): string {
  switch (status) {
    case 'pending':
      return t('digitalAvatar.governance.ticketStatus.pending', '待审批');
    case 'approved':
      return t('digitalAvatar.governance.ticketStatus.approved', '已通过');
    case 'rejected':
      return t('digitalAvatar.governance.ticketStatus.rejected', '已拒绝');
    case 'experimenting':
      return t('digitalAvatar.governance.ticketStatus.experimenting', '实验中');
    case 'deployed':
      return t('digitalAvatar.governance.ticketStatus.deployed', '已部署');
    case 'rolled_back':
      return t('digitalAvatar.governance.ticketStatus.rolled_back', '已回滚');
    default:
      return status;
  }
}

function getRuntimeStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: RuntimeLogStatus | string,
): string {
  switch (status) {
    case 'pending':
      return t('digitalAvatar.governance.runtimeStatus.pending', '待处理');
    case 'ticketed':
      return t('digitalAvatar.governance.runtimeStatus.ticketed', '已转工单');
    case 'requested':
      return t('digitalAvatar.governance.runtimeStatus.requested', '已转提权');
    case 'dismissed':
      return t('digitalAvatar.governance.runtimeStatus.dismissed', '已忽略');
    default:
      return status;
  }
}

function getGovernanceQueueStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  kind: string,
  status: string,
): string {
  if (kind === 'proposal') return getProposalStatusText(t, status);
  if (kind === 'ticket') return getOptimizationStatusText(t, status);
  return getGovernanceStatusText(t, status);
}

function getTimelineRowStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  rowType: GovernanceRowType,
  status: string,
): string {
  if (rowType === 'runtime') return getRuntimeStatusText(t, status);
  if (rowType === 'proposal') return getProposalStatusText(t, status);
  if (rowType === 'ticket') return getOptimizationStatusText(t, status);
  return getGovernanceStatusText(t, status);
}

function getRuntimeSeverityText(
  t: ReturnType<typeof useTranslation>['t'],
  severity: 'error' | 'warn' | 'info',
): string {
  switch (severity) {
    case 'error':
      return t('digitalAvatar.governance.runtimeSeverity.error', '错误');
    case 'warn':
      return t('digitalAvatar.governance.runtimeSeverity.warn', '警告');
    case 'info':
      return t('digitalAvatar.governance.runtimeSeverity.info', '正常');
    default:
      return severity;
  }
}

function eventSeverity(event: ChatSessionEvent): 'error' | 'warn' | 'info' {
  const payload = event.payload || {};
  if (event.event_type === 'toolresult' && payload.success === false) return 'error';
  if (event.event_type === 'done' && typeof payload.error === 'string' && payload.error.trim()) return 'error';
  if (event.event_type === 'status') {
    const status = String(payload.status || '').toLowerCase();
    if (status.includes('error') || status.includes('fail') || status.includes('timeout')) return 'error';
    if (status.includes('retry') || status.includes('compaction')) return 'warn';
  }
  if (['thinking', 'turn', 'compaction'].includes(event.event_type)) return 'warn';
  return 'info';
}

function eventSummary(event: ChatSessionEvent): string {
  const payload = event.payload || {};
  if (typeof payload.content === 'string' && payload.content.trim()) return payload.content.trim();
  if (typeof payload.preview === 'string' && payload.preview.trim()) return payload.preview.trim();
  if (typeof payload.error === 'string' && payload.error.trim()) return payload.error.trim();
  if (typeof payload.status === 'string' && payload.status.trim()) return payload.status.trim();
  if (typeof payload.name === 'string' && payload.name.trim()) return payload.name.trim();
  try {
    return JSON.stringify(payload);
  } catch {
    return '';
  }
}

function persistedEventKey(event: ChatSessionEvent): string {
  return `${event.run_id || 'no_run'}:${event.event_id}:${event.created_at}`;
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

function mergePersistedEvents(base: ChatSessionEvent[], incoming: ChatSessionEvent[], mode: PersistedEventLoadMode): ChatSessionEvent[] {
  if (incoming.length === 0) return base;
  const merged = mode === 'older' ? [...incoming, ...base] : [...base, ...incoming];
  const dedup = new Map<string, ChatSessionEvent>();
  for (const item of merged) dedup.set(persistedEventKey(item), item);
  return Array.from(dedup.values()).sort((a, b) => {
    if (a.event_id !== b.event_id) return a.event_id - b.event_id;
    return Date.parse(a.created_at) - Date.parse(b.created_at);
  });
}

function isOpenGovernanceStatus(status: string): boolean {
  return [
    'pending',
    'pending_approval',
    'needs_human',
    'approved',
    'pilot',
    'experimenting',
    'ticketed',
    'requested',
    'active',
  ].includes(status);
}

function toDecisionStatus(decision: DecisionMode): 'pending' | 'approved' | 'needs_human' | 'rejected' {
  if (decision === 'approve_direct' || decision === 'approve_sandbox') return 'approved';
  if (decision === 'require_human_confirm') return 'needs_human';
  return 'rejected';
}

export default function DigitalAvatarTimelinePage() {
  const { t } = useTranslation();
  const { teamId, avatarId } = useParams<{ teamId: string; avatarId: string }>();
  const navigate = useNavigate();
  const { addToast } = useToast();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [avatar, setAvatar] = useState<PortalDetail | null>(null);
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [governance, setGovernance] = useState<AvatarGovernanceState>(createEmptyGovernanceState());
  const [governanceEvents, setGovernanceEvents] = useState<AvatarGovernanceEventPayload[]>([]);
  const [governanceQueue, setGovernanceQueue] = useState<AvatarGovernanceQueueItemPayload[]>([]);
  const [managerSessionId, setManagerSessionId] = useState<string | null>(null);
  const governanceRef = useRef<AvatarGovernanceState>(createEmptyGovernanceState());
  const [governanceSaving, setGovernanceSaving] = useState(false);
  const [queueKindFilter, setQueueKindFilter] = useState<QueueKindFilter>('all');
  const [queueStateFilter, setQueueStateFilter] = useState<ReviewStateFilter>('open');
  const [queueRiskFilter, setQueueRiskFilter] = useState<RiskFilter>('all');
  const [queueSearch, setQueueSearch] = useState('');
  const [selectedQueueIds, setSelectedQueueIds] = useState<string[]>([]);
  const [timelineKindFilter, setTimelineKindFilter] = useState<TimelineKindFilter>('all');
  const [timelineStateFilter, setTimelineStateFilter] = useState<ReviewStateFilter>('all');
  const [timelineRiskFilter, setTimelineRiskFilter] = useState<RiskFilter>('all');
  const [timelineActorFilter, setTimelineActorFilter] = useState('all');
  const [timelineSearch, setTimelineSearch] = useState('');
  const [batchSummary, setBatchSummary] = useState<BatchGovernanceSummary | null>(null);
  const [persistedEvents, setPersistedEvents] = useState<ChatSessionEvent[]>([]);
  const persistedEventsRef = useRef<ChatSessionEvent[]>([]);
  const [persistedEventsLoading, setPersistedEventsLoading] = useState(false);
  const [persistedEventsLoadingMore, setPersistedEventsLoadingMore] = useState(false);
  const [persistedEventsHasMore, setPersistedEventsHasMore] = useState(false);
  const [persistedEventsError, setPersistedEventsError] = useState<string | null>(null);
  const [persistedEventFilter, setPersistedEventFilter] = useState<PersistedEventFilter>('all');
  const [persistedEventSearch, setPersistedEventSearch] = useState('');
  const persistedOldestEventIdRef = useRef<number | null>(null);
  const persistedLatestEventIdRef = useRef<number | null>(null);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored !== null ? stored === 'true' : false;
  });

  const canManage = Boolean(team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin');

  const managerAgent = useMemo(() => {
    const id = avatar?.codingAgentId || avatar?.agentId || null;
    return agents.find(agent => agent.id === id) || null;
  }, [agents, avatar?.codingAgentId, avatar?.agentId]);

  const serviceAgent = useMemo(() => {
    const id = avatar?.serviceAgentId || avatar?.agentId || null;
    return agents.find(agent => agent.id === id) || null;
  }, [agents, avatar?.serviceAgentId, avatar?.agentId]);
  const avatarType = useMemo(
    () => (avatar ? detectAvatarType(avatar) : 'unknown'),
    [avatar],
  );

  const managerSessionStorageKey = useMemo(() => (
    teamId && avatarId ? `digital_avatar_manager_session:v1:${teamId}:${avatarId}` : ''
  ), [teamId, avatarId]);

  const handleSectionChange = useCallback((section: string) => {
    if (!teamId) return;
    navigate(`/teams/${teamId}?section=${section}`);
  }, [navigate, teamId]);

  const handleToggleSidebar = useCallback(() => {
    setSidebarCollapsed(prev => {
      localStorage.setItem(STORAGE_KEY, String(!prev));
      return !prev;
    });
  }, []);

  useEffect(() => {
    governanceRef.current = governance;
  }, [governance]);

  const loadPersistedRuntimeEvents = useCallback(async (
    sessionId: string,
    options?: { mode?: PersistedEventLoadMode; silent?: boolean },
  ) => {
    const mode = options?.mode || 'latest';
    const silent = options?.silent ?? false;
    if (mode === 'older') setPersistedEventsLoadingMore(true);
    else if (!silent) setPersistedEventsLoading(true);
    setPersistedEventsError(null);
    try {
      const query: Parameters<typeof chatApi.listSessionEvents>[1] = {
        runId: '__all__',
        limit: PERSISTED_EVENTS_PAGE_SIZE,
      };
      if (mode === 'latest') {
        query.order = 'desc';
      } else if (mode === 'older') {
        const beforeId = persistedOldestEventIdRef.current;
        if (!beforeId || beforeId <= 0) {
          setPersistedEventsHasMore(false);
          return;
        }
        query.order = 'desc';
        query.beforeEventId = beforeId;
      } else {
        const afterId = persistedLatestEventIdRef.current;
        if (!afterId || afterId <= 0) return;
        query.order = 'asc';
        query.afterEventId = afterId;
      }
      const fetched = await chatApi.listSessionEvents(sessionId, query);
      const normalized = mode === 'latest' || mode === 'older' ? fetched.slice().reverse() : fetched;
      const base = mode === 'latest' ? [] : persistedEventsRef.current;
      const merged = mode === 'latest' ? normalized : mergePersistedEvents(base, normalized, mode);
      persistedEventsRef.current = merged;
      setPersistedEvents(merged);
      if (merged.length > 0) {
        persistedOldestEventIdRef.current = merged[0].event_id;
        persistedLatestEventIdRef.current = merged[merged.length - 1].event_id;
      } else {
        persistedOldestEventIdRef.current = null;
        persistedLatestEventIdRef.current = null;
      }
      if (mode === 'latest' || mode === 'older') {
        setPersistedEventsHasMore(fetched.length >= PERSISTED_EVENTS_PAGE_SIZE);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('common.error');
      setPersistedEventsError(msg);
    } finally {
      if (mode === 'older') setPersistedEventsLoadingMore(false);
      else if (!silent) setPersistedEventsLoading(false);
    }
  }, [t]);

  const loadData = useCallback(async () => {
    if (!teamId || !avatarId) return;
    try {
      setLoading(true);
      const [teamResult, avatarResult, governanceResult, eventsResult, queueResult, agentResult] = await Promise.all([
        apiClient.getTeam(teamId),
        avatarPortalApi.get(teamId, avatarId),
        avatarPortalApi.getGovernance(teamId, avatarId).catch(() => null),
        avatarPortalApi.listGovernanceEvents(teamId, avatarId, 300).catch(() => []),
        avatarPortalApi.listGovernanceQueue(teamId, avatarId).catch(() => []),
        agentApi.listAgents(teamId, 1, 300).catch(() => ({ items: [] as TeamAgent[], total: 0, page: 1, limit: 300, totalPages: 1 })),
      ]);
      setTeam(teamResult.team);
      setAvatar(avatarResult);
      setGovernance(governanceResult ? readGovernanceState({ digitalAvatarGovernance: governanceResult.state }) : readGovernanceState(avatarResult.settings || {}));
      setGovernanceEvents(eventsResult);
      setGovernanceQueue(queueResult);
      setAgents(agentResult.items || []);
      try {
        setManagerSessionId(window.localStorage.getItem(managerSessionStorageKey) || null);
      } catch {
        setManagerSessionId(null);
      }
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [avatarId, managerSessionStorageKey, t, teamId]);

  useEffect(() => {
    setSelectedQueueIds((current) => current.filter((id) => governanceQueue.some((item) => item.id === id)));
  }, [governanceQueue]);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  useEffect(() => {
    if (!managerSessionId) {
      setPersistedEvents([]);
      persistedEventsRef.current = [];
      persistedOldestEventIdRef.current = null;
      persistedLatestEventIdRef.current = null;
      setPersistedEventsHasMore(false);
      setPersistedEventsError(null);
      return;
    }
    void loadPersistedRuntimeEvents(managerSessionId, { mode: 'latest', silent: false });
  }, [loadPersistedRuntimeEvents, managerSessionId]);

  const governanceStats = useMemo(() => ({
    queue: governanceQueue.length,
    events: governanceEvents.length,
    runtimeLogs: governance.runtimeLogs.length,
    pendingRuntimeLogs: governance.runtimeLogs.filter(item => item.status === 'pending').length,
  }), [governance.runtimeLogs, governanceEvents.length, governanceQueue.length]);

  const governanceTimelineRows = useMemo(() => {
    if (governanceEvents.length > 0) {
      return governanceEvents
        .map((event) => ({
          id: event.event_id || `${event.entity_type}:${event.entity_id || event.created_at}`,
          ts: event.created_at,
          rowType: event.entity_type === 'runtime'
            ? 'runtime'
            : event.entity_type === 'capability'
            ? 'capability'
            : event.entity_type === 'proposal'
            ? 'proposal'
            : 'ticket',
          title: event.title,
          detail: event.detail || '',
          status: event.status || event.event_type,
          meta: [event.event_type ? `事件:${event.event_type}` : '', event.actor_name ? `执行人:${event.actor_name}` : ''].filter(Boolean),
        }) as GovernanceTimelineRow)
        .sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts));
    }

    const rows: GovernanceTimelineRow[] = [];
    governance.runtimeLogs.forEach((item) => {
      rows.push({ id: `runtime:${item.id}`, ts: item.createdAt, rowType: 'runtime', title: item.title, detail: item.proposal || item.evidence || '', status: item.status, meta: [item.problemType, item.risk].filter(Boolean) });
    });
    governance.capabilityRequests.forEach((item) => {
      rows.push({ id: `capability:${item.id}`, ts: item.updatedAt || item.createdAt, rowType: 'capability', title: item.title, detail: item.detail || item.decisionReason || '', status: item.status, meta: [item.risk, item.source].filter(Boolean) });
    });
    governance.gapProposals.forEach((item) => {
      rows.push({ id: `proposal:${item.id}`, ts: item.updatedAt || item.createdAt, rowType: 'proposal', title: item.title, detail: item.description || '', status: item.status, meta: [item.expectedGain].filter(Boolean) });
    });
    governance.optimizationTickets.forEach((item) => {
      rows.push({ id: `ticket:${item.id}`, ts: item.updatedAt || item.createdAt, rowType: 'ticket', title: item.title, detail: item.proposal || item.evidence || '', status: item.status, meta: [item.problemType, item.risk].filter(Boolean) });
    });
    return rows.sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts));
  }, [governance, governanceEvents]);

  const governanceAuditRows = useMemo(() => governanceTimelineRows.filter(item => item.rowType !== 'runtime'), [governanceTimelineRows]);

  const timelineActorOptions = useMemo(() => {
    const actors = new Set<string>();
    governanceEvents.forEach((event) => {
      const actor = (event.actor_name || '').trim();
      if (actor) actors.add(actor);
    });
    return Array.from(actors).sort((a, b) => a.localeCompare(b));
  }, [governanceEvents]);

  const extractRiskFromTexts = useCallback((values: string[]): 'low' | 'medium' | 'high' | null => {
    const joined = values.join(' ').toLowerCase();
    if (joined.includes('high')) return 'high';
    if (joined.includes('medium')) return 'medium';
    if (joined.includes('low')) return 'low';
    return null;
  }, []);

  const visibleGovernanceQueue = useMemo(() => {
    const keyword = queueSearch.trim().toLowerCase();
    return governanceQueue.filter((item) => {
      if (queueKindFilter !== 'all' && item.kind !== queueKindFilter) return false;
      if (queueStateFilter === 'open' && !isOpenGovernanceStatus(item.status)) return false;
      if (queueStateFilter === 'resolved' && isOpenGovernanceStatus(item.status)) return false;
      if (queueRiskFilter !== 'all') {
        const risk = extractRiskFromTexts(item.meta);
        if (risk !== queueRiskFilter) return false;
      }
      if (!keyword) return true;
      const text = `${item.title} ${item.detail} ${item.meta.join(' ')}`.toLowerCase();
      return text.includes(keyword);
    });
  }, [extractRiskFromTexts, governanceQueue, queueKindFilter, queueRiskFilter, queueSearch, queueStateFilter]);

  const selectedQueueItems = useMemo(() => {
    const selected = new Set(selectedQueueIds);
    return governanceQueue.filter((item) => selected.has(item.id));
  }, [governanceQueue, selectedQueueIds]);

  const selectedQueueKinds = useMemo(() => {
    return Array.from(new Set(selectedQueueItems.map((item) => item.kind)));
  }, [selectedQueueItems]);

  const selectedQueueKind = selectedQueueKinds.length === 1 ? selectedQueueKinds[0] : null;

  const visibleGovernanceTimelineRows = useMemo(() => {
    const keyword = timelineSearch.trim().toLowerCase();
    return governanceTimelineRows.filter((item) => {
      if (timelineKindFilter !== 'all' && item.rowType !== timelineKindFilter) return false;
      if (timelineStateFilter === 'open' && !isOpenGovernanceStatus(item.status)) return false;
      if (timelineStateFilter === 'resolved' && isOpenGovernanceStatus(item.status)) return false;
      if (timelineRiskFilter !== 'all') {
        const risk = extractRiskFromTexts(item.meta);
        if (risk !== timelineRiskFilter) return false;
      }
      if (timelineActorFilter !== 'all') {
        const actorMatch = item.meta.some((meta) => meta === `执行人:${timelineActorFilter}`);
        if (!actorMatch) return false;
      }
      if (!keyword) return true;
      const text = `${item.title} ${item.detail} ${item.meta.join(' ')}`.toLowerCase();
      return text.includes(keyword);
    });
  }, [extractRiskFromTexts, governanceTimelineRows, timelineActorFilter, timelineKindFilter, timelineRiskFilter, timelineSearch, timelineStateFilter]);

  const persistGovernance = useCallback(async (
    updater: (current: AvatarGovernanceState) => AvatarGovernanceState,
    successMessage?: string,
  ) => {
    if (!teamId || !avatar) return;
    setGovernanceSaving(true);
    try {
      const nextState = updater(governanceRef.current);
      const payload = await avatarPortalApi.updateGovernance(teamId, avatar.id, {
        state: nextState as unknown as Record<string, unknown>,
      });
      const nextGovernance = readGovernanceState({ digitalAvatarGovernance: payload.state });
      setGovernance(nextGovernance);
      governanceRef.current = nextGovernance;
      setAvatar((current) => current ? ({
        ...current,
        settings: {
          ...(current.settings || {}),
          digitalAvatarGovernance: payload.state,
        },
      }) : current);
      const [eventsResult, queueResult] = await Promise.all([
        avatarPortalApi.listGovernanceEvents(teamId, avatar.id, 300).catch(() => []),
        avatarPortalApi.listGovernanceQueue(teamId, avatar.id).catch(() => []),
      ]);
      setGovernanceEvents(eventsResult);
      setGovernanceQueue(queueResult);
      setSelectedQueueIds([]);
      addToast('success', successMessage || t('common.saved'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setGovernanceSaving(false);
    }
  }, [addToast, avatar, t, teamId]);

  const decideCapabilityRequest = useCallback(async (ids: string[], decision: DecisionMode) => {
    if (!ids.length) return;
    const idSet = new Set(ids);
    const now = new Date().toISOString();
    await persistGovernance((current) => ({
      ...current,
      capabilityRequests: current.capabilityRequests.map((item) => (
        idSet.has(item.id)
          ? {
              ...item,
              status: toDecisionStatus(decision),
              decision,
              updatedAt: now,
            }
          : item
      )),
    }), t('digitalAvatar.timeline.batchApplied', '已更新治理状态'));
    setBatchSummary({
      kind: 'capability',
      count: ids.length,
      actionLabel:
        decision === 'approve_direct'
          ? t('digitalAvatar.governance.action.approve', '通过')
          : decision === 'approve_sandbox'
          ? t('digitalAvatar.governance.action.sandbox', '沙箱通过')
          : decision === 'require_human_confirm'
          ? t('digitalAvatar.governance.action.human', '转人工')
          : t('digitalAvatar.governance.action.reject', '拒绝'),
      updatedAt: now,
    });
  }, [persistGovernance, t]);

  const updateGapProposalStatus = useCallback(async (ids: string[], status: ProposalStatus) => {
    if (!ids.length) return;
    const idSet = new Set(ids);
    const now = new Date().toISOString();
    await persistGovernance((current) => ({
      ...current,
      gapProposals: current.gapProposals.map((item) => (
        idSet.has(item.id)
          ? { ...item, status, updatedAt: now }
          : item
      )),
    }), t('digitalAvatar.timeline.batchApplied', '已更新治理状态'));
    setBatchSummary({
      kind: 'proposal',
      count: ids.length,
      actionLabel: t(`digitalAvatar.governance.proposalStatus.${status}` as const, status),
      updatedAt: now,
    });
  }, [persistGovernance, t]);

  const updateOptimizationStatus = useCallback(async (ids: string[], status: OptimizationStatus) => {
    if (!ids.length) return;
    const idSet = new Set(ids);
    const now = new Date().toISOString();
    await persistGovernance((current) => ({
      ...current,
      optimizationTickets: current.optimizationTickets.map((item) => (
        idSet.has(item.id)
          ? { ...item, status, updatedAt: now }
          : item
      )),
    }), t('digitalAvatar.timeline.batchApplied', '已更新治理状态'));
    setBatchSummary({
      kind: 'ticket',
      count: ids.length,
      actionLabel: t(`digitalAvatar.governance.ticketStatus.${status}` as const, status),
      updatedAt: now,
    });
  }, [persistGovernance, t]);

  const toggleQueueSelection = useCallback((id: string) => {
    setSelectedQueueIds((current) => current.includes(id)
      ? current.filter((item) => item !== id)
      : [...current, id]);
  }, []);

  const toggleSelectVisibleQueue = useCallback(() => {
    const visibleIds = visibleGovernanceQueue.map((item) => item.id);
    setSelectedQueueIds((current) => {
      const allSelected = visibleIds.length > 0 && visibleIds.every((id) => current.includes(id));
      if (allSelected) {
        return current.filter((id) => !visibleIds.includes(id));
      }
      return Array.from(new Set([...current, ...visibleIds]));
    });
  }, [visibleGovernanceQueue]);

  const jumpToManagerWorkspace = useCallback((text?: string) => {
    if (!teamId || !avatarId) return;
    try {
      window.localStorage.setItem(`${MANAGER_FOCUS_STORAGE_PREFIX}${teamId}`, avatarId);
      if (text?.trim()) {
        window.localStorage.setItem(`${MANAGER_COMPOSE_STORAGE_PREFIX}${teamId}:${avatarId}`, JSON.stringify({
          id: `timeline_prompt_${Date.now()}`,
          text: text.trim(),
          autoSend: true,
        }));
      }
    } catch {
      // ignore storage failures
    }
    navigate(`/teams/${teamId}?section=digital-avatar`);
  }, [avatarId, navigate, teamId]);

  const sendSelectedQueueToManager = useCallback(() => {
    if (!selectedQueueItems.length || !avatar) return;
    const prompt = [
      `请处理数字分身治理事项。portal_id=${avatar.id}，名称=${avatar.name}，slug=${avatar.slug}。`,
      '要求：先判断风险、权限边界与最小变更范围，再给执行建议；必要时直接执行低风险动作，并回报结果、风险与回滚建议。',
      '待处理事项：',
      ...selectedQueueItems.map((item, index) => `- ${index + 1}. [${item.kind}/${item.status}] ${item.title}：${item.detail}`),
    ].join('\n');
    jumpToManagerWorkspace(prompt);
  }, [avatar, jumpToManagerWorkspace, selectedQueueItems]);
  const visiblePersistedEvents = useMemo(() => {
    const keyword = persistedEventSearch.trim().toLowerCase();
    return persistedEvents.filter((event) => {
      if (persistedEventFilter === 'error' && eventSeverity(event) !== 'error') return false;
      if (persistedEventFilter === 'tool' && !['toolcall', 'toolresult'].includes(event.event_type)) return false;
      if (persistedEventFilter === 'thinking' && !['thinking', 'turn', 'compaction'].includes(event.event_type)) return false;
      if (persistedEventFilter === 'status' && !['status', 'done', 'workspace_changed'].includes(event.event_type)) return false;
      if (!keyword) return true;
      const text = `${event.event_type} ${eventSummary(event)}`.toLowerCase();
      return text.includes(keyword);
    });
  }, [persistedEventFilter, persistedEventSearch, persistedEvents]);
  const exportAuditSummary = useCallback(() => {
    if (!avatar) return;
    const queueKindLabel = queueKindFilter === 'all'
      ? t('digitalAvatar.timeline.filterAll', '全部')
      : queueKindFilter === 'capability'
      ? t('digitalAvatar.governance.queueType.capability', '提权')
      : queueKindFilter === 'proposal'
      ? t('digitalAvatar.governance.queueType.proposal', '新分身')
      : t('digitalAvatar.governance.queueType.ticket', '优化');
    const timelineKindLabel = timelineKindFilter === 'all'
      ? t('digitalAvatar.timeline.filterAll', '全部')
      : timelineKindFilter === 'runtime'
      ? t('digitalAvatar.governance.timelineType.runtime', '运行')
      : timelineKindFilter === 'capability'
      ? t('digitalAvatar.governance.timelineType.capability', '提权')
      : timelineKindFilter === 'proposal'
      ? t('digitalAvatar.governance.timelineType.proposal', '新分身')
      : t('digitalAvatar.governance.timelineType.ticket', '优化');
    const queueLines = visibleGovernanceQueue.slice(0, 20).map((item, index) => [
      `${index + 1}. [${item.kind}/${item.status}] ${item.title}`,
      `   - ${t('common.description', '描述')}: ${item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}`,
      `   - ${t('common.time', '时间')}: ${formatDateTime(item.ts)}`,
      item.meta.length ? `   - Meta: ${item.meta.join(' | ')}` : '',
    ].filter(Boolean).join('\n'));
    const auditLines = governanceAuditRows.slice(0, 20).map((item, index) => [
      `${index + 1}. [${item.rowType}/${item.status}] ${item.title}`,
      `   - ${t('common.description', '描述')}: ${item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}`,
      `   - ${t('common.time', '时间')}: ${formatDateTime(item.ts)}`,
    ].join('\n'));
    const timelineLines = visibleGovernanceTimelineRows.slice(0, 30).map((item, index) => [
      `${index + 1}. [${item.rowType}/${item.status}] ${item.title}`,
      `   - ${t('common.description', '描述')}: ${item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}`,
      `   - ${t('common.time', '时间')}: ${formatDateTime(item.ts)}`,
      item.meta.length ? `   - Meta: ${item.meta.join(' | ')}` : '',
    ].filter(Boolean).join('\n'));
    const runtimeLines = visiblePersistedEvents.slice(-20).reverse().map((event, index) => [
      `${index + 1}. [${event.event_type}/${eventSeverity(event)}] #${event.event_id}`,
      `   - ${t('common.description', '描述')}: ${eventSummary(event) || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}`,
      `   - ${t('common.time', '时间')}: ${formatDateTime(event.created_at)}`,
      `   - run_id: ${event.run_id || 'run:unknown'}`,
    ].join('\n'));
    const approvedCount = governanceAuditRows.filter((item) => ['approved', 'active', 'pilot', 'deployed'].includes(item.status)).length;
    const needsHumanCount = governanceAuditRows.filter((item) => item.status === 'needs_human').length;
    const rejectedCount = governanceAuditRows.filter((item) => ['rejected', 'rolled_back', 'failed'].includes(item.status)).length;
    const runtimeCount = governanceTimelineRows.filter((item) => item.rowType === 'runtime').length;
    const content = [
      `# ${t('digitalAvatar.timeline.exportTitle', '数字分身治理审计摘要')}`,
      '',
      `- ${t('digitalAvatar.timeline.exportGeneratedAt', '生成时间')}: ${formatDateTime(new Date().toISOString())}`,
      `- ${t('common.name', '名称')}: ${avatar.name}`,
      `- Slug: /p/${avatar.slug}`,
      `- ${t('digitalAvatar.labels.managerAgent', '管理 Agent')}: ${managerAgent?.name || '-'}`,
      `- ${t('digitalAvatar.labels.serviceAgent', '分身 Agent')}: ${serviceAgent?.name || '-'}`,
      `- ${t('digitalAvatar.workspace.summaryAccess', '文档模式')}: ${avatar.documentAccessMode}`,
      '',
      `## ${t('digitalAvatar.timeline.exportFilterSnapshot', '当前筛选快照')}`,
      `- ${t('digitalAvatar.governance.queueTitle', '治理队列')}: ${queueKindLabel} / ${queueStateFilter} / ${queueRiskFilter}${queueSearch ? ` / ${queueSearch}` : ''}`,
      `- ${t('digitalAvatar.timeline.pageTitle', '治理时间线')}: ${timelineKindLabel} / ${timelineStateFilter} / ${timelineRiskFilter} / ${timelineActorFilter}${timelineSearch ? ` / ${timelineSearch}` : ''}`,
      '',
      `## ${t('digitalAvatar.timeline.exportDecisionSnapshot', '审批结论摘要')}`,
      `- ${t('digitalAvatar.governance.pendingSectionTitle', '待处理治理事项')}: ${governanceQueue.filter((item) => isOpenGovernanceStatus(item.status)).length}`,
      `- ${t('digitalAvatar.governance.resolvedSectionTitle', '已处理事项')}: ${governanceQueue.filter((item) => !isOpenGovernanceStatus(item.status)).length}`,
      `- ${t('digitalAvatar.governance.autoSectionTitle', '自动治理记录')}: ${runtimeCount}`,
      `- ${t('digitalAvatar.timeline.exportApprovedCount', '已通过/已生效')}: ${approvedCount}`,
      `- ${t('digitalAvatar.timeline.exportNeedsHumanCount', '需人工确认')}: ${needsHumanCount}`,
      `- ${t('digitalAvatar.timeline.exportRejectedCount', '已拒绝/已回滚')}: ${rejectedCount}`,
      ...(batchSummary
        ? [`- ${t('digitalAvatar.timeline.exportRecentBatch', '最近一次批量治理')}: ${batchSummary.actionLabel} · ${batchSummary.count} · ${formatDateTime(batchSummary.updatedAt)}`]
        : []),
      '',
      `## ${t('digitalAvatar.governance.pendingSectionTitle', '待处理治理事项')} (${visibleGovernanceQueue.length})`,
      ...(queueLines.length ? queueLines : [`- ${t('digitalAvatar.governance.pendingSectionEmpty', '当前没有待处理治理事项')}`]),
      '',
      `## ${t('digitalAvatar.governance.decisionAuditTitle', '治理决策审计')} (${governanceAuditRows.length})`,
      ...(auditLines.length ? auditLines : [`- ${t('digitalAvatar.governance.decisionAuditEmpty', '暂无决策记录')}`]),
      '',
      `## ${t('digitalAvatar.timeline.pageTitle', '治理时间线')} (${visibleGovernanceTimelineRows.length})`,
      ...(timelineLines.length ? timelineLines : [`- ${t('digitalAvatar.governance.timelineEmpty', '暂无摘要记录')}`]),
      '',
      `## ${t('digitalAvatar.governance.runtimeEventsTitle', '完整运行日志（可追溯）')} (${visiblePersistedEvents.length})`,
      ...(runtimeLines.length ? runtimeLines : [`- ${t('digitalAvatar.governance.runtimeEventsEmpty', '暂无可展示事件')}`]),
      '',
      `## ${t('digitalAvatar.timeline.exportSummaryFooter', '备注')}`,
      `- ${t('digitalAvatar.timeline.exportSummaryFooterHint', '本摘要仅导出当前筛选结果，用于审批留档、治理复盘和跨团队同步。')}`,
    ].join('\n');
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-');
    downloadTextFile(`${avatar.slug}-governance-audit-${stamp}.md`, content);
    addToast('success', t('digitalAvatar.timeline.exportDone', '审计摘要已导出'));
  }, [
    addToast,
    avatar,
    governanceAuditRows,
    managerAgent?.name,
    queueKindFilter,
    queueRiskFilter,
    queueSearch,
    queueStateFilter,
    serviceAgent?.name,
    t,
    timelineActorFilter,
    timelineKindFilter,
    timelineRiskFilter,
    timelineSearch,
    timelineStateFilter,
    batchSummary,
    governanceQueue,
    visibleGovernanceQueue,
    visibleGovernanceTimelineRows,
    visiblePersistedEvents,
    governanceTimelineRows,
  ]);

  if (loading) {
    return (
      <AppShell className="team-font-cap">
        <div className="space-y-4">
          <Skeleton className="h-12 w-64" />
          <Skeleton className="h-36 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      </AppShell>
    );
  }

  if (!team || !avatar || error) {
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
              {t('digitalAvatar.timeline.backToWorkspace', '返回数字分身工作台')}
            </Button>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" onClick={() => void loadData()}>
                <RefreshCw className="mr-1.5 h-4 w-4" />
                {t('common.refresh', '刷新')}
              </Button>
              <Button variant="outline" size="sm" onClick={exportAuditSummary}>
                <Download className="mr-1.5 h-4 w-4" />
                {t('digitalAvatar.timeline.exportAction', '导出审计摘要')}
              </Button>
              <Button variant="outline" size="sm" onClick={() => jumpToManagerWorkspace()}>
                {t('digitalAvatar.timeline.backToManager', '回到管理 Agent 对话')}
              </Button>
              <Button variant="outline" size="sm" disabled={!avatar.publicUrl} onClick={() => avatar.publicUrl && window.open(avatar.publicUrl, '_blank', 'noopener,noreferrer')}>
                <ExternalLink className="mr-1.5 h-4 w-4" />
                {t('digitalAvatar.workspace.openPublicPage', '打开访客页')}
              </Button>
            </div>
          </div>

          <Card className="border-border/70">
            <CardHeader className="pb-3">
              <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
                <div className="min-w-0 space-y-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <CardTitle className="text-xl">{avatar.name}</CardTitle>
                    <Badge variant="outline" className="text-[11px]">/p/{avatar.slug}</Badge>
                    <StatusBadge status={PORTAL_STATUS_MAP[avatar.status] || 'neutral'}>{getAvatarPortalStatusText(t, avatar.status)}</StatusBadge>
                    <AvatarTypeBadge type={avatarType} />
                  </div>
                  <p className="text-sm text-muted-foreground">{avatar.description || t('digitalAvatar.timeline.descriptionFallback', '独立查看这个数字分身的治理轨迹、审批变化与管理 Agent 执行记录。')}</p>
                </div>
                <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground xl:w-[min(32vw,340px)] xl:max-w-[340px]">
                  <div>
                    <div>{t('digitalAvatar.labels.managerAgent', '管理 Agent')}</div>
                    <div className="mt-1 flex flex-wrap items-center gap-1.5 font-medium text-foreground">
                      {managerAgent ? <AgentTypeBadge type={resolveAgentVisualType(managerAgent)} /> : null}
                      <span>{managerAgent?.name || '-'}</span>
                    </div>
                  </div>
                  <div>
                    <div>{t('digitalAvatar.labels.serviceAgent', '分身 Agent')}</div>
                    <div className="mt-1 flex items-center gap-2 font-medium text-foreground">
                      {serviceAgent ? <AgentTypeBadge type={resolveAgentVisualType(serviceAgent)} /> : null}
                      <span>{serviceAgent?.name || '-'}</span>
                      {serviceAgent ? <StatusBadge status={AGENT_STATUS_MAP[serviceAgent.status] || 'neutral'}>{t(`agent.status.${serviceAgent.status}`, serviceAgent.status)}</StatusBadge> : null}
                    </div>
                  </div>
                  <div><div>{t('digitalAvatar.workspace.summaryAccess', '文档模式')}</div><div className="mt-1 font-medium text-foreground">{getDigitalAvatarDocumentAccessModeText(t, avatar.documentAccessMode)}</div></div>
                  <div><div>{t('digitalAvatar.timeline.lastUpdated', '最近更新')}</div><div className="mt-1 font-medium text-foreground">{formatRelativeTime(avatar.updatedAt)}</div></div>
                </div>
              </div>
            </CardHeader>
            <CardContent className="grid gap-3 md:grid-cols-4">
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.timeline.queueCount', '治理队列')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{governanceStats.queue}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.timeline.eventCount', '治理事件')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{governanceStats.events}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.timeline.runtimeIssueCount', '运行建议')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{governanceStats.runtimeLogs}</div></div>
              <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4"><div className="text-xs text-muted-foreground">{t('digitalAvatar.timeline.pendingRuntimeIssueCount', '待处理运行建议')}</div><div className="mt-2 text-2xl font-semibold text-foreground">{governanceStats.pendingRuntimeLogs}</div></div>
            </CardContent>
          </Card>

          {batchSummary ? (
            <Card className="border-border/70 bg-primary/5">
              <CardContent className="flex flex-wrap items-center justify-between gap-3 py-4">
                <div className="space-y-1">
                  <p className="text-sm font-medium text-foreground">
                    {t('digitalAvatar.timeline.lastBatchSummaryTitle', '最近一次批量治理')}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t('digitalAvatar.timeline.lastBatchSummaryDetail', '已对 {{count}} 个{{kind}}执行“{{action}}”', {
                      count: batchSummary.count,
                      kind:
                        batchSummary.kind === 'capability'
                          ? t('digitalAvatar.governance.queueType.capability', '提权')
                          : batchSummary.kind === 'proposal'
                          ? t('digitalAvatar.governance.queueType.proposal', '新分身')
                          : t('digitalAvatar.governance.queueType.ticket', '优化'),
                      action: batchSummary.actionLabel,
                    })}
                  </p>
                </div>
                <div className="text-[11px] text-muted-foreground">{formatDateTime(batchSummary.updatedAt)}</div>
              </CardContent>
            </Card>
          ) : null}

          <div className="grid gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
            <div className="space-y-6">
              <Card className="border-border/70">
                <CardHeader className="pb-2"><CardTitle className="text-base flex items-center gap-1.5"><ShieldAlert className="h-4 w-4" />{t('digitalAvatar.governance.queueTitle', '治理队列')}</CardTitle></CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="relative min-w-[140px] flex-1 sm:min-w-[180px]">
                      <Filter className="pointer-events-none absolute left-2 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
                      <Input className="h-8 pl-7 text-xs" placeholder={t('digitalAvatar.timeline.queueSearch', '搜索治理事项')} value={queueSearch} onChange={(event) => setQueueSearch(event.target.value)} />
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['all', 'capability', 'proposal', 'ticket'] as QueueKindFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${queueKindFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setQueueKindFilter(filter)}>
                          {filter === 'all'
                            ? t('digitalAvatar.timeline.filterAll', '全部')
                            : filter === 'capability'
                            ? t('digitalAvatar.governance.queueType.capability', '提权')
                            : filter === 'proposal'
                            ? t('digitalAvatar.governance.queueType.proposal', '新分身')
                            : t('digitalAvatar.governance.queueType.ticket', '优化')}
                        </button>
                      ))}
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['all', 'low', 'medium', 'high'] as RiskFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${queueRiskFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setQueueRiskFilter(filter)}>
                          {filter === 'all'
                            ? t('digitalAvatar.timeline.filterAllRisk', '全部风险')
                            : t(`digitalAvatar.timeline.risk.${filter}` as const, filter)}
                        </button>
                      ))}
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['open', 'resolved', 'all'] as ReviewStateFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${queueStateFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setQueueStateFilter(filter)}>
                          {filter === 'open'
                            ? t('digitalAvatar.timeline.filterOpen', '待处理')
                            : filter === 'resolved'
                            ? t('digitalAvatar.timeline.filterResolved', '已处理')
                            : t('digitalAvatar.timeline.filterAll', '全部')}
                        </button>
                      ))}
                    </div>
                  </div>
                  {canManage && selectedQueueItems.length > 0 ? (
                    <div className="rounded-lg border border-primary/30 bg-primary/5 p-3 space-y-2">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div className="text-xs text-muted-foreground">
                          {t('digitalAvatar.timeline.selectionSummary', '已选择 {{count}} 项治理事项', { count: selectedQueueItems.length })}
                        </div>
                        <div className="flex items-center gap-2">
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" onClick={sendSelectedQueueToManager}>
                            {t('digitalAvatar.timeline.sendSelectedToManager', '交给管理 Agent')}
                          </Button>
                          <Button size="sm" variant="ghost" className="h-7 px-2 text-[11px]" onClick={toggleSelectVisibleQueue}>
                            {visibleGovernanceQueue.length > 0 && visibleGovernanceQueue.every((item) => selectedQueueIds.includes(item.id))
                              ? t('digitalAvatar.timeline.clearVisibleSelection', '取消当前筛选')
                              : t('digitalAvatar.timeline.selectVisible', '选中当前筛选')}
                          </Button>
                          <Button size="sm" variant="ghost" className="h-7 px-2 text-[11px]" onClick={() => setSelectedQueueIds([])}>
                            {t('digitalAvatar.timeline.clearSelection', '清空选择')}
                          </Button>
                        </div>
                      </div>
                      {selectedQueueKind === 'capability' ? (
                        <div className="flex flex-wrap gap-1">
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void decideCapabilityRequest(selectedQueueItems.map((item) => item.source_id), 'approve_direct')}>{t('digitalAvatar.governance.action.approve', '通过')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void decideCapabilityRequest(selectedQueueItems.map((item) => item.source_id), 'approve_sandbox')}>{t('digitalAvatar.governance.action.sandbox', '沙箱通过')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void decideCapabilityRequest(selectedQueueItems.map((item) => item.source_id), 'require_human_confirm')}>{t('digitalAvatar.governance.action.human', '转人工')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void decideCapabilityRequest(selectedQueueItems.map((item) => item.source_id), 'deny')}>{t('digitalAvatar.governance.action.reject', '拒绝')}</Button>
                        </div>
                      ) : null}
                      {selectedQueueKind === 'proposal' ? (
                        <div className="flex flex-wrap gap-1">
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateGapProposalStatus(selectedQueueItems.map((item) => item.source_id), 'approved')}>{t('digitalAvatar.governance.action.approve', '通过')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateGapProposalStatus(selectedQueueItems.map((item) => item.source_id), 'pilot')}>{t('digitalAvatar.governance.action.pilot', '试运行')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateGapProposalStatus(selectedQueueItems.map((item) => item.source_id), 'active')}>{t('digitalAvatar.governance.action.active', '生效')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateGapProposalStatus(selectedQueueItems.map((item) => item.source_id), 'rejected')}>{t('digitalAvatar.governance.action.reject', '拒绝')}</Button>
                        </div>
                      ) : null}
                      {selectedQueueKind === 'ticket' ? (
                        <div className="flex flex-wrap gap-1">
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateOptimizationStatus(selectedQueueItems.map((item) => item.source_id), 'approved')}>{t('digitalAvatar.governance.action.approve', '通过')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateOptimizationStatus(selectedQueueItems.map((item) => item.source_id), 'experimenting')}>{t('digitalAvatar.governance.action.experiment', '实验')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateOptimizationStatus(selectedQueueItems.map((item) => item.source_id), 'deployed')}>{t('digitalAvatar.governance.action.deploy', '部署')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateOptimizationStatus(selectedQueueItems.map((item) => item.source_id), 'rolled_back')}>{t('digitalAvatar.governance.action.rollback', '回滚')}</Button>
                          <Button size="sm" variant="outline" className="h-7 px-2 text-[11px]" disabled={governanceSaving} onClick={() => void updateOptimizationStatus(selectedQueueItems.map((item) => item.source_id), 'rejected')}>{t('digitalAvatar.governance.action.reject', '拒绝')}</Button>
                        </div>
                      ) : null}
                      {!selectedQueueKind ? (
                        <div className="text-[11px] text-muted-foreground">{t('digitalAvatar.timeline.batchMixedHint', '批量操作仅支持同类型治理事项，请先筛选或只选择同类项。')}</div>
                      ) : null}
                    </div>
                  ) : null}
                  {visibleGovernanceQueue.length === 0 ? <p className="text-sm text-muted-foreground">{t('digitalAvatar.governance.queueEmpty', '当前没有待处理治理事项')}</p> : visibleGovernanceQueue.map(item => (
                    <div key={item.id} className="rounded-lg border border-border/70 p-3">
                      <div className="flex items-center justify-between gap-2">
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            {canManage ? (
                              <input type="checkbox" className="h-3.5 w-3.5 accent-primary" checked={selectedQueueIds.includes(item.id)} onChange={() => toggleQueueSelection(item.id)} />
                            ) : null}
                            <Badge variant="outline" className="text-[10px]">{item.kind === 'capability' ? t('digitalAvatar.governance.queueType.capability', '提权') : item.kind === 'proposal' ? t('digitalAvatar.governance.queueType.proposal', '新分身') : t('digitalAvatar.governance.queueType.ticket', '优化')}</Badge>
                            <p className="truncate text-sm font-medium">{item.title}</p>
                          </div>
                          <p className="mt-1 text-xs text-muted-foreground">{item.detail}</p>
                        </div>
                        <span className={`rounded border px-1.5 py-0.5 text-[10px] ${badgeClass(item.status)}`}>{getGovernanceQueueStatusText(t, item.kind, item.status)}</span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                        <span>{formatDateTime(item.ts)}</span>
                        {item.meta.map(meta => <span key={meta}>{formatDigitalAvatarMetaLabel(t, meta)}</span>)}
                      </div>
                    </div>
                  ))}
                </CardContent>
              </Card>

              <Card className="border-border/70">
                <CardHeader className="pb-2"><CardTitle className="text-base flex items-center gap-1.5"><Activity className="h-4 w-4" />{t('digitalAvatar.timeline.pageTitle', '治理时间线')}</CardTitle></CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="relative min-w-[140px] flex-1 sm:min-w-[180px]">
                      <Filter className="pointer-events-none absolute left-2 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
                      <Input className="h-8 pl-7 text-xs" placeholder={t('digitalAvatar.timeline.timelineSearch', '搜索时间线记录')} value={timelineSearch} onChange={(event) => setTimelineSearch(event.target.value)} />
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['all', 'runtime', 'capability', 'proposal', 'ticket'] as TimelineKindFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${timelineKindFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setTimelineKindFilter(filter)}>
                          {filter === 'all'
                            ? t('digitalAvatar.timeline.filterAll', '全部')
                            : filter === 'runtime'
                            ? t('digitalAvatar.governance.timelineType.runtime', '运行')
                            : filter === 'capability'
                            ? t('digitalAvatar.governance.timelineType.capability', '提权')
                            : filter === 'proposal'
                            ? t('digitalAvatar.governance.timelineType.proposal', '新分身')
                            : t('digitalAvatar.governance.timelineType.ticket', '优化')}
                        </button>
                      ))}
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['all', 'low', 'medium', 'high'] as RiskFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${timelineRiskFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setTimelineRiskFilter(filter)}>
                          {filter === 'all'
                            ? t('digitalAvatar.timeline.filterAllRisk', '全部风险')
                            : t(`digitalAvatar.timeline.risk.${filter}` as const, filter)}
                        </button>
                      ))}
                    </div>
                    <div className="flex flex-wrap gap-1">
                      <select className="h-7 rounded border border-border/60 bg-background px-2 text-[11px] text-muted-foreground" value={timelineActorFilter} onChange={(event) => setTimelineActorFilter(event.target.value)}>
                        <option value="all">{t('digitalAvatar.timeline.filterAllActors', '全部执行人')}</option>
                        {timelineActorOptions.map((actor) => (
                          <option key={actor} value={actor}>{actor}</option>
                        ))}
                      </select>
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['open', 'resolved', 'all'] as ReviewStateFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${timelineStateFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setTimelineStateFilter(filter)}>
                          {filter === 'open'
                            ? t('digitalAvatar.timeline.filterOpen', '待处理')
                            : filter === 'resolved'
                            ? t('digitalAvatar.timeline.filterResolved', '已处理')
                            : t('digitalAvatar.timeline.filterAll', '全部')}
                        </button>
                      ))}
                    </div>
                  </div>
                  {visibleGovernanceTimelineRows.length === 0 ? <p className="text-sm text-muted-foreground">{t('digitalAvatar.governance.timelineEmpty', '暂无摘要记录')}</p> : visibleGovernanceTimelineRows.map(item => (
                    <div key={item.id} className={`rounded-lg border p-3 ${item.rowType === 'runtime' ? 'border-status-warning/35 bg-status-warning/10' : 'border-border/70 bg-muted/10'}`}>
                      <div className="flex items-center justify-between gap-2">
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <Badge variant="outline" className="text-[10px]">{item.rowType === 'runtime' ? t('digitalAvatar.governance.timelineType.runtime', '运行') : item.rowType === 'capability' ? t('digitalAvatar.governance.timelineType.capability', '提权') : item.rowType === 'proposal' ? t('digitalAvatar.governance.timelineType.proposal', '新分身') : t('digitalAvatar.governance.timelineType.ticket', '优化')}</Badge>
                            <p className="truncate text-sm font-medium">{item.title}</p>
                          </div>
                          <p className="mt-1 text-xs text-muted-foreground">{item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}</p>
                        </div>
                        <span className={`rounded border px-1.5 py-0.5 text-[10px] ${item.rowType === 'runtime' ? runtimeStatusClass(item.status as RuntimeLogStatus) : badgeClass(item.status)}`}>{getTimelineRowStatusText(t, item.rowType, item.status)}</span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
                        <span>{formatDateTime(item.ts)}</span>
                        {item.meta.map(meta => <span key={meta}>{formatDigitalAvatarMetaLabel(t, meta)}</span>)}
                        <button type="button" className="rounded border border-border/60 px-1.5 py-0.5 text-[10px] hover:bg-muted" onClick={() => jumpToManagerWorkspace(`请针对数字分身“${avatar.name}”(portal_id=${avatar.id}) 处理这条治理记录：\n类型=${item.rowType}\n状态=${item.status}\n标题=${item.title}\n详情=${item.detail || '无'}\n补充=${item.meta.join(' / ') || '无'}\n要求：先判断风险和最小执行路径，再给出执行结果、风险与回滚建议。`)}>
                          {t('digitalAvatar.timeline.handleInManager', '交给管理 Agent')}
                        </button>
                      </div>
                    </div>
                  ))}
                </CardContent>
              </Card>
            </div>

            <div className="space-y-6">
              <Card className="border-border/70">
                <CardHeader className="pb-2"><CardTitle className="text-base">{t('digitalAvatar.governance.decisionAuditTitle', '治理决策审计')}</CardTitle></CardHeader>
                <CardContent className="space-y-2">
                  {governanceAuditRows.length === 0 ? <p className="text-sm text-muted-foreground">{t('digitalAvatar.governance.decisionAuditEmpty', '暂无决策记录')}</p> : governanceAuditRows.slice(0, 12).map(item => (
                    <div key={item.id} className="rounded-lg border border-border/70 bg-muted/10 p-3">
                      <div className="flex items-center justify-between gap-2"><p className="truncate text-sm font-medium">{item.title}</p><span className={`rounded border px-1.5 py-0.5 text-[10px] ${badgeClass(item.status)}`}>{getTimelineRowStatusText(t, item.rowType, item.status)}</span></div>
                      <p className="mt-1 text-xs text-muted-foreground">{item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}</p>
                      <div className="mt-2 text-[11px] text-muted-foreground">{formatDateTime(item.ts)}</div>
                    </div>
                  ))}
                </CardContent>
              </Card>

              <Card className="border-border/70">
                <CardHeader className="pb-2">
                  <CardTitle className="text-base flex items-center justify-between gap-2">
                    <span>{t('digitalAvatar.governance.runtimeEventsTitle', '完整运行日志（可追溯）')}</span>
                    <Button size="sm" variant="ghost" className="h-7 px-2 text-[11px]" disabled={!persistedEventsHasMore || persistedEventsLoadingMore || persistedEventsLoading} onClick={() => managerSessionId && loadPersistedRuntimeEvents(managerSessionId, { mode: 'older', silent: false })}>
                      {persistedEventsLoadingMore ? <Loader2 className="h-3 w-3 animate-spin" /> : t('digitalAvatar.governance.runtimeEventsLoadOlder', '加载更早')}
                    </Button>
                  </CardTitle>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="relative min-w-[140px] flex-1 sm:min-w-[180px]">
                      <Filter className="pointer-events-none absolute left-2 top-2.5 h-3.5 w-3.5 text-muted-foreground" />
                      <Input className="h-8 pl-7 text-xs" placeholder={t('digitalAvatar.governance.runtimeEventsSearch', '搜索事件内容')} value={persistedEventSearch} onChange={(e) => setPersistedEventSearch(e.target.value)} />
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {(['all', 'error', 'tool', 'thinking', 'status'] as PersistedEventFilter[]).map((filter) => (
                        <button key={filter} type="button" className={`rounded border px-2 py-1 text-[11px] ${persistedEventFilter === filter ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border/60 bg-background text-muted-foreground'}`} onClick={() => setPersistedEventFilter(filter)}>
                          {t(`digitalAvatar.governance.runtimeEventsFilter.${filter}`, filter)}
                        </button>
                      ))}
                    </div>
                  </div>
                  {persistedEventsError ? <div className="rounded border border-status-error/40 bg-status-error/10 px-2 py-1 text-[11px] text-status-error-text">{persistedEventsError}</div> : null}
                  {!managerSessionId ? <p className="text-sm text-muted-foreground">{t('digitalAvatar.governance.runtimeEventsNoSession', '暂无可追溯会话，请先与管理 Agent 开始对话。')}</p> : persistedEventsLoading ? <div className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />{t('common.loading', '加载中')}</div> : visiblePersistedEvents.length === 0 ? <p className="text-sm text-muted-foreground">{t('digitalAvatar.governance.runtimeEventsEmpty', '暂无可展示事件')}</p> : (
                    <div className="space-y-2 max-h-[720px] overflow-y-auto pr-1">
                      {visiblePersistedEvents.map((event) => {
                        const severity = eventSeverity(event);
                        return (
                          <div key={persistedEventKey(event)} className={`rounded-lg border p-3 ${severity === 'error' ? 'border-status-error/35 bg-status-error/10' : severity === 'warn' ? 'border-status-warning/35 bg-status-warning/10' : 'border-border/70 bg-muted/10'}`}>
                            <div className="flex items-center justify-between gap-2">
                              <div className="min-w-0">
                                <p className="truncate text-sm font-medium">#{event.event_id} · {event.event_type}</p>
                                <p className="mt-0.5 text-[11px] text-muted-foreground truncate">{event.run_id || 'run:unknown'} · {formatDateTime(event.created_at)}</p>
                              </div>
                              <span className={`rounded border px-1.5 py-0.5 text-[10px] ${badgeClass(severity === 'error' ? 'rejected' : severity === 'warn' ? 'pending' : 'approved')}`}>{getRuntimeSeverityText(t, severity)}</span>
                            </div>
                            <p className="mt-2 whitespace-pre-wrap break-words text-xs text-muted-foreground">{eventSummary(event) || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}</p>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </CardContent>
              </Card>
            </div>
          </div>
        </div>
      </AppShell>
    </TeamProvider>
  );
}
