import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  Bot,
  Brain,
  Check,
  CircleSlash,
  Clock3,
  ExternalLink,
  Loader2,
  Plus,
  RefreshCw,
  ShieldAlert,
  Sparkles,
  UserRound,
  Users,
} from 'lucide-react';
import { Button } from '../ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { portalApi, type PortalDetail, type PortalSummary } from '../../api/portal';
import { chatApi, type ChatSessionEvent } from '../../api/chat';
import { agentApi, type TeamAgent } from '../../api/agent';
import { ChatConversation, type ChatRuntimeEvent } from '../chat/ChatConversation';
import type { ChatInputComposeRequest } from '../chat/ChatInput';
import { useToast } from '../../contexts/ToastContext';
import { CreateAvatarDialog } from './digital-avatar/CreateAvatarDialog';
import { CreateManagerAgentDialog } from './digital-avatar/CreateManagerAgentDialog';
import { DigitalAvatarGuide } from './digital-avatar/DigitalAvatarGuide';
import {
  createEmptyGovernanceState,
  makeId,
  mergeGovernanceAutomationConfig,
  mergeGovernanceSettings,
  readGovernanceAutomationConfig,
  readGovernanceState,
  type AgentGapProposal,
  type AvatarGovernanceAutomationConfig,
  type AvatarGovernanceState,
  type CapabilityGapRequest,
  type DecisionMode,
  type OptimizationStatus,
  type OptimizationTicket,
  type ProposalStatus,
  type RuntimeLogEntry,
  type RuntimeLogStatus,
} from './digital-avatar/governance';
import {
  getDigitalAvatarManagerId,
  isDigitalAvatarPortal,
  splitGeneralAndDedicatedAgents,
} from './agentIsolation';

interface DigitalAvatarSectionProps {
  teamId: string;
  canManage: boolean;
}

type AvatarFilter = 'all' | 'external' | 'internal';
type WorkspaceTab = 'workspace' | 'guide';
type RuntimeLogFilter = 'pending' | 'all';
type PersistedEventFilter = 'all' | 'error' | 'tool' | 'thinking' | 'status';
type RuntimeSuggestion = RuntimeLogEntry;
type PersistedEventLoadMode = 'latest' | 'older' | 'incremental';

function isAvatar(summary: PortalSummary): boolean {
  return isDigitalAvatarPortal(summary);
}

function detectAvatarType(summary: PortalSummary): 'external' | 'internal' | 'unknown' {
  if ((summary.tags || []).includes('avatar:external')) return 'external';
  if ((summary.tags || []).includes('avatar:internal')) return 'internal';
  return 'unknown';
}

function resolveManagerGroupCandidates(agents: TeamAgent[], avatars: PortalSummary[]): TeamAgent[] {
  const { managerDedicatedAgents } = splitGeneralAndDedicatedAgents(agents, avatars);
  return managerDedicatedAgents;
}

function getAgentName(agents: TeamAgent[], id: string | null | undefined, fallback = 'N/A'): string {
  if (!id) return fallback;
  return agents.find(a => a.id === id)?.name || id;
}

function toIsoNow(): string {
  return new Date().toISOString();
}

function toRisk(score: number): 'low' | 'medium' | 'high' {
  if (score >= 0.75) return 'high';
  if (score >= 0.35) return 'medium';
  return 'low';
}

function toDecisionStatus(decision: DecisionMode): CapabilityGapRequest['status'] {
  if (decision === 'approve_direct' || decision === 'approve_sandbox') return 'approved';
  if (decision === 'require_human_confirm') return 'needs_human';
  return 'rejected';
}

function toProposalStatusLabel(status: ProposalStatus): string {
  switch (status) {
    case 'pending_approval':
      return 'pending_approval';
    case 'approved':
      return 'approved';
    case 'rejected':
      return 'rejected';
    case 'pilot':
      return 'pilot';
    case 'active':
      return 'active';
    default:
      return 'draft';
  }
}

function toOptimizationStatusLabel(status: OptimizationStatus): string {
  switch (status) {
    case 'approved':
    case 'rejected':
    case 'experimenting':
    case 'deployed':
    case 'rolled_back':
      return status;
    default:
      return 'pending';
  }
}

interface RuntimeSuggestionText {
  unknownTool: string;
  toolFailureTitle: (tool: string) => string;
  toolFailureEvidenceFallback: string;
  toolFailureProposal: (tool: string) => string;
  toolFailureGain: string;
  sessionFailedTitle: string;
  sessionFailedProposal: string;
  sessionFailedGain: string;
}

const MAX_RUNTIME_LOGS = 80;
const PERSISTED_EVENTS_PAGE_SIZE = 200;

interface GovernanceExecutionBinding {
  id: string;
  entityType: 'capability' | 'proposal' | 'ticket';
  targetId: string;
  targetStatus: string;
}

interface GovernanceActionReceipt {
  actionId: string;
  outcome: 'success' | 'partial' | 'failed';
  summary?: string;
  reason?: string;
}

function persistedEventKey(event: ChatSessionEvent): string {
  return `${event.run_id || 'no_run'}:${event.event_id}:${event.created_at}`;
}

function mergePersistedEvents(
  base: ChatSessionEvent[],
  incoming: ChatSessionEvent[],
  mode: PersistedEventLoadMode,
): ChatSessionEvent[] {
  if (incoming.length === 0) return base;
  const merged = mode === 'older' ? [...incoming, ...base] : [...base, ...incoming];
  const dedup = new Map<string, ChatSessionEvent>();
  for (const item of merged) {
    dedup.set(persistedEventKey(item), item);
  }
  return Array.from(dedup.values()).sort((a, b) => {
    if (a.event_id !== b.event_id) return a.event_id - b.event_id;
    return Date.parse(a.created_at) - Date.parse(b.created_at);
  });
}

function isRuntimeDoneFailure(detail: Record<string, unknown> | undefined): boolean {
  const error = typeof detail?.error === 'string' ? detail.error.trim() : '';
  if (error) return true;
  const status = typeof detail?.status === 'string' ? detail.status.trim().toLowerCase() : '';
  if (!status) return false;
  return status.includes('fail') || status.includes('error') || status.includes('timeout') || status.includes('cancel');
}

function parseGovernanceActionReceipts(text: string): GovernanceActionReceipt[] {
  if (!text.trim()) return [];
  const receipts: GovernanceActionReceipt[] = [];
  const tagPattern = /<governance_action_result>([\s\S]*?)<\/governance_action_result>/gi;
  let match: RegExpExecArray | null;
  while ((match = tagPattern.exec(text)) !== null) {
    const raw = (match[1] || '').trim();
    if (!raw) continue;
    try {
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      const actionId = String(parsed.action_id || parsed.actionId || '').trim();
      const outcomeRaw = String(parsed.outcome || '').trim().toLowerCase();
      const outcome = outcomeRaw === 'success' || outcomeRaw === 'partial' || outcomeRaw === 'failed'
        ? (outcomeRaw as GovernanceActionReceipt['outcome'])
        : null;
      if (!actionId || !outcome) continue;
      receipts.push({
        actionId,
        outcome,
        summary: typeof parsed.summary === 'string' ? parsed.summary.trim() : undefined,
        reason: typeof parsed.reason === 'string' ? parsed.reason.trim() : undefined,
      });
    } catch {
      // Ignore malformed result block.
    }
  }
  return receipts;
}

function summarizeRuntimeFailure(
  event: ChatRuntimeEvent,
  text: RuntimeSuggestionText,
): RuntimeSuggestion | null {
  const detail = event.detail || {};
  if (event.kind === 'toolresult') {
    const success = detail.success;
    if (success !== false) return null;
    const toolName = typeof detail.toolName === 'string' && detail.toolName.trim()
      ? detail.toolName.trim()
      : text.unknownTool;
    const preview = typeof detail.preview === 'string' ? detail.preview.trim() : '';
    const evidence = preview || event.text || text.toolFailureEvidenceFallback;
    const risk = toRisk(preview.length > 120 ? 0.65 : 0.4);
    return {
      id: makeId('runtime'),
      title: text.toolFailureTitle(toolName),
      evidence,
      proposal: text.toolFailureProposal(toolName),
      expectedGain: text.toolFailureGain,
      risk,
      problemType: 'tool',
      createdAt: toIsoNow(),
      status: 'pending',
    };
  }
  if (event.kind === 'done' && typeof detail.error === 'string' && detail.error.trim()) {
    const error = detail.error.trim();
    return {
      id: makeId('runtime'),
      title: text.sessionFailedTitle,
      evidence: error,
      proposal: text.sessionFailedProposal,
      expectedGain: text.sessionFailedGain,
      risk: 'medium',
      problemType: 'policy',
      createdAt: toIsoNow(),
      status: 'pending',
    };
  }
  return null;
}

function badgeClass(status: string): string {
  if (status === 'approved' || status === 'deployed' || status === 'active') {
    return 'bg-status-success/15 text-status-success-text border-status-success/40';
  }
  if (status === 'rejected' || status === 'deny' || status === 'rolled_back') {
    return 'bg-status-error/15 text-status-error-text border-status-error/40';
  }
  if (status === 'pending' || status === 'pending_approval' || status === 'needs_human') {
    return 'bg-status-warning/15 text-status-warning-text border-status-warning/40';
  }
  return 'bg-muted text-muted-foreground border-border/60';
}

function runtimeStatusClass(status: RuntimeLogStatus): string {
  if (status === 'pending') {
    return 'bg-status-warning/15 text-status-warning-text border-status-warning/40';
  }
  if (status === 'ticketed' || status === 'requested') {
    return 'bg-status-success/15 text-status-success-text border-status-success/40';
  }
  return 'bg-muted text-muted-foreground border-border/60';
}

function eventSeverity(event: ChatSessionEvent): 'error' | 'warn' | 'info' {
  if (event.event_type === 'done') {
    const payload = event.payload || {};
    const errorText = typeof payload.error === 'string' ? payload.error.trim() : '';
    const status = typeof payload.status === 'string' ? payload.status.toLowerCase() : '';
    if (errorText || status === 'failed' || status === 'error') return 'error';
    return 'info';
  }
  if (event.event_type === 'toolresult') {
    const success = (event.payload || {}).success;
    return success === false ? 'error' : 'info';
  }
  if (event.event_type === 'status') {
    const raw = String((event.payload || {}).status || '').toLowerCase();
    if (raw.includes('error') || raw.includes('failed') || raw.includes('timeout')) return 'warn';
    return 'info';
  }
  return 'info';
}

function eventTypeBadge(eventType: string): string {
  if (eventType === 'toolcall' || eventType === 'toolresult') return 'tool';
  if (eventType === 'thinking' || eventType === 'turn' || eventType === 'compaction') return 'thinking';
  if (eventType === 'status' || eventType === 'done' || eventType === 'workspace_changed') return 'status';
  return eventType;
}

function eventSummary(event: ChatSessionEvent): string {
  const payload = event.payload || {};
  if (event.event_type === 'text' || event.event_type === 'thinking') {
    return String(payload.content || '').trim();
  }
  if (event.event_type === 'status') {
    return String(payload.status || '').trim();
  }
  if (event.event_type === 'toolcall') {
    const name = String(payload.name || '').trim();
    const id = String(payload.id || '').trim();
    return name ? `${name}${id ? ` (${id})` : ''}` : id;
  }
  if (event.event_type === 'toolresult') {
    const name = String(payload.name || '').trim();
    const id = String(payload.id || '').trim();
    const success = payload.success === false ? 'failed' : 'ok';
    const content = String(payload.content || '').trim();
    const label = name || id || 'tool';
    return `${label}: ${success}${content ? ` · ${content}` : ''}`;
  }
  if (event.event_type === 'done') {
    const status = String(payload.status || '').trim();
    const error = String(payload.error || '').trim();
    return error ? `${status || 'done'} · ${error}` : status || 'done';
  }
  if (event.event_type === 'turn') {
    return `${String(payload.current || '')}/${String(payload.max || '')}`;
  }
  return JSON.stringify(payload);
}

function severityClass(severity: 'error' | 'warn' | 'info'): string {
  if (severity === 'error') return 'border-status-error/40 bg-status-error/10';
  if (severity === 'warn') return 'border-status-warning/40 bg-status-warning/10';
  return 'border-border/70 bg-muted/20';
}

export function DigitalAvatarSection({ teamId, canManage }: DigitalAvatarSectionProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const navigate = useNavigate();

  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [tab, setTab] = useState<WorkspaceTab>('workspace');
  const [filter, setFilter] = useState<AvatarFilter>('all');
  const [avatars, setAvatars] = useState<PortalSummary[]>([]);
  const [selectedAvatarId, setSelectedAvatarId] = useState<string | null>(null);
  const [selectedAvatar, setSelectedAvatar] = useState<PortalDetail | null>(null);
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [governance, setGovernance] = useState<AvatarGovernanceState>(createEmptyGovernanceState());
  const governanceRef = useRef(governance);
  const selectedAvatarRef = useRef<PortalDetail | null>(selectedAvatar);
  const governancePersistQueueRef = useRef<Promise<void>>(Promise.resolve());
  const governancePersistInFlightRef = useRef(0);
  const [savingGovernance, setSavingGovernance] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [createManagerOpen, setCreateManagerOpen] = useState(false);
  const [managerSessionId, setManagerSessionId] = useState<string | null>(null);
  const [managerProcessing, setManagerProcessing] = useState(false);
  const [runtimeLogFilter, setRuntimeLogFilter] = useState<RuntimeLogFilter>('pending');
  const [bootstrapManagerAgentId, setBootstrapManagerAgentId] = useState('');
  const [managerComposeRequest, setManagerComposeRequest] = useState<ChatInputComposeRequest | null>(null);
  const [autoProposalTriggerCountDraft, setAutoProposalTriggerCountDraft] = useState(3);
  const [savingAutomationConfig, setSavingAutomationConfig] = useState(false);
  const [persistedEvents, setPersistedEvents] = useState<ChatSessionEvent[]>([]);
  const persistedEventsRef = useRef<ChatSessionEvent[]>([]);
  const [persistedEventsLoading, setPersistedEventsLoading] = useState(false);
  const [persistedEventsLoadingMore, setPersistedEventsLoadingMore] = useState(false);
  const [persistedEventsError, setPersistedEventsError] = useState<string | null>(null);
  const [persistedEventFilter, setPersistedEventFilter] = useState<PersistedEventFilter>('all');
  const [persistedEventSearch, setPersistedEventSearch] = useState('');
  const [persistedEventsHasMore, setPersistedEventsHasMore] = useState(false);
  const persistedOldestEventIdRef = useRef<number | null>(null);
  const persistedLatestEventIdRef = useRef<number | null>(null);
  const scheduledGovernanceExecutionRef = useRef<GovernanceExecutionBinding[]>([]);
  const inflightGovernanceExecutionRef = useRef<GovernanceExecutionBinding[]>([]);
  const handledGovernanceActionIdsRef = useRef<Set<string>>(new Set());
  const runtimeAssistantBufferRef = useRef('');

  const sessionStoragePrefix = `digital_avatar_manager_session:v1:${teamId}:`;

  const managerAgentId = selectedAvatar?.codingAgentId || selectedAvatar?.agentId || null;
  const managerGroupOptions = useMemo(
    () => resolveManagerGroupCandidates(agents, avatars),
    [agents, avatars],
  );

  const fallbackManagerAgentId = managerGroupOptions[0]?.id || null;
  const selectedManagerGroupId = bootstrapManagerAgentId || fallbackManagerAgentId;
  const managerScopedAvatars = useMemo(() => {
    const scopeManagerId = selectedAvatar
      ? (selectedAvatar.codingAgentId || selectedAvatar.agentId || null)
      : selectedManagerGroupId;
    if (!scopeManagerId) return avatars;
    return avatars.filter((avatar) => getDigitalAvatarManagerId(avatar) === scopeManagerId);
  }, [avatars, selectedAvatar, selectedManagerGroupId]);

  const visibleAvatars = useMemo(() => {
    const base = managerScopedAvatars;
    if (filter === 'all') return base;
    return base.filter((avatar) => detectAvatarType(avatar) === filter);
  }, [filter, managerScopedAvatars]);
  const managerGroupStats = useMemo(() => {
    const total = managerScopedAvatars.length;
    const external = managerScopedAvatars.filter((avatar) => detectAvatarType(avatar) === 'external').length;
    const internal = managerScopedAvatars.filter((avatar) => detectAvatarType(avatar) === 'internal').length;
    return { total, external, internal };
  }, [managerScopedAvatars]);

  const effectiveManagerAgentId = selectedAvatar
    ? managerAgentId
    : selectedManagerGroupId;
  const managerAgent = useMemo(
    () => agents.find(agent => agent.id === effectiveManagerAgentId) || null,
    [agents, effectiveManagerAgentId]
  );

  const serviceAgentId = selectedAvatar?.serviceAgentId || selectedAvatar?.agentId || null;

  const runtimePendingCount = useMemo(
    () => governance.runtimeLogs.filter((item) => item.status === 'pending').length,
    [governance.runtimeLogs]
  );

  const automationConfig = useMemo<AvatarGovernanceAutomationConfig>(() => {
    return readGovernanceAutomationConfig(
      selectedAvatar?.settings as Record<string, unknown> | null | undefined,
    );
  }, [selectedAvatar?.settings]);

  useEffect(() => {
    setAutoProposalTriggerCountDraft(automationConfig.autoProposalTriggerCount);
  }, [automationConfig.autoProposalTriggerCount]);

  const visibleRuntimeSuggestions = useMemo(
    () =>
      runtimeLogFilter === 'pending'
        ? governance.runtimeLogs.filter((item) => item.status === 'pending')
        : governance.runtimeLogs,
    [governance.runtimeLogs, runtimeLogFilter]
  );

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

  const governanceAuditRows = useMemo(() => {
    const rows: Array<{
      id: string;
      ts: string;
      type: 'capability' | 'proposal' | 'ticket';
      title: string;
      status: string;
      detail: string;
    }> = [];

    governance.capabilityRequests.forEach((item) => {
      if (item.status === 'pending' && !item.decision) return;
      rows.push({
        id: `capability:${item.id}`,
        ts: item.updatedAt || item.createdAt,
        type: 'capability',
        title: item.title,
        status: item.status,
        detail: item.decisionReason || item.detail || '',
      });
    });
    governance.gapProposals.forEach((item) => {
      if (item.status === 'draft') return;
      rows.push({
        id: `proposal:${item.id}`,
        ts: item.updatedAt || item.createdAt,
        type: 'proposal',
        title: item.title,
        status: item.status,
        detail: item.description || '',
      });
    });
    governance.optimizationTickets.forEach((item) => {
      if (item.status === 'pending') return;
      rows.push({
        id: `ticket:${item.id}`,
        ts: item.updatedAt || item.createdAt,
        type: 'ticket',
        title: item.title,
        status: item.status,
        detail: item.proposal || item.evidence || '',
      });
    });

    rows.sort((a, b) => {
      const ta = Date.parse(a.ts);
      const tb = Date.parse(b.ts);
      return (Number.isFinite(tb) ? tb : 0) - (Number.isFinite(ta) ? ta : 0);
    });
    return rows;
  }, [governance.capabilityRequests, governance.gapProposals, governance.optimizationTickets]);

  const runtimeSuggestionText = useMemo<RuntimeSuggestionText>(() => ({
    unknownTool: t('digitalAvatar.governance.runtimeUnknownTool', '未知工具'),
    toolFailureTitle: (tool: string) =>
      t('digitalAvatar.governance.runtimeToolFailureTitle', '工具执行失败：{{tool}}', { tool }),
    toolFailureEvidenceFallback: t(
      'digitalAvatar.governance.runtimeToolFailureEvidenceFallback',
      '工具执行失败，未返回详细预览。'
    ),
    toolFailureProposal: (tool: string) =>
      t(
        'digitalAvatar.governance.runtimeToolFailureProposal',
        '检查 {{tool}} 的权限边界、输入契约与回退路径，并补充有停止条件的受控重试策略。',
        { tool }
      ),
    toolFailureGain: t(
      'digitalAvatar.governance.runtimeToolFailureGain',
      '降低重复工具失败率，提升任务完成稳定性。'
    ),
    sessionFailedTitle: t('digitalAvatar.governance.runtimeSessionFailedTitle', '会话终止失败'),
    sessionFailedProposal: t(
      'digitalAvatar.governance.runtimeSessionFailedProposal',
      '复核任务提示词与策略约束，补充最小失败恢复流程。'
    ),
    sessionFailedGain: t(
      'digitalAvatar.governance.runtimeSessionFailedGain',
      '降低硬失败中断，提升成功交付率。'
    ),
  }), [t]);

  useEffect(() => {
    governanceRef.current = governance;
  }, [governance]);

  useEffect(() => {
    selectedAvatarRef.current = selectedAvatar;
  }, [selectedAvatar]);

  useEffect(() => {
    persistedEventsRef.current = persistedEvents;
  }, [persistedEvents]);

  const loadAvatars = useCallback(async (withLoading = true) => {
    try {
      if (withLoading) setLoading(true);
      setRefreshing(true);
      const [portalRes, agentRes] = await Promise.all([
        portalApi.list(teamId, 1, 200, 'avatar'),
        agentApi.listAgents(teamId, 1, 200),
      ]);
      const avatarItems = (portalRes.items || []).filter(isAvatar);
      const nextAgents = agentRes.items || [];
      const managerGroups = resolveManagerGroupCandidates(nextAgents, avatarItems);
      setAvatars(avatarItems);
      setAgents(nextAgents);
      setBootstrapManagerAgentId((prev) => {
        if (prev && managerGroups.some(agent => agent.id === prev)) return prev;
        return managerGroups[0]?.id || '';
      });
      setSelectedAvatarId((prev) => {
        if (prev && avatarItems.some(item => item.id === prev)) return prev;
        return null;
      });
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('digitalAvatar.states.loading'));
    } finally {
      if (withLoading) setLoading(false);
      setRefreshing(false);
    }
  }, [addToast, t, teamId]);

  const loadAvatarDetail = useCallback(async (avatarId: string) => {
    try {
      const detail = await portalApi.get(teamId, avatarId);
      setSelectedAvatar(detail);
      setGovernance(readGovernanceState(detail.settings));
      setRuntimeLogFilter('pending');
      try {
        const saved = window.localStorage.getItem(`${sessionStoragePrefix}${avatarId}`);
        setManagerSessionId(saved || null);
      } catch {
        setManagerSessionId(null);
      }
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
      setSelectedAvatar(null);
      setGovernance(createEmptyGovernanceState());
      setManagerSessionId(null);
      setRuntimeLogFilter('pending');
    }
  }, [addToast, teamId, t, sessionStoragePrefix]);

  useEffect(() => {
    loadAvatars(true);
  }, [loadAvatars]);

  useEffect(() => {
    if (!selectedAvatarId) {
      setSelectedAvatar(null);
      setGovernance(createEmptyGovernanceState());
      setManagerSessionId(null);
      setRuntimeLogFilter('pending');
      return;
    }
    loadAvatarDetail(selectedAvatarId);
  }, [loadAvatarDetail, selectedAvatarId]);

  useEffect(() => {
    if (!selectedAvatarId) return;
    if (visibleAvatars.some((avatar) => avatar.id === selectedAvatarId)) return;
    setSelectedAvatarId(visibleAvatars[0]?.id || null);
  }, [selectedAvatarId, visibleAvatars]);

  useEffect(() => {
    if (selectedAvatarId) return;
    if (visibleAvatars.length === 0) return;
    setSelectedAvatarId(visibleAvatars[0].id);
  }, [selectedAvatarId, visibleAvatars]);

  useEffect(() => {
    scheduledGovernanceExecutionRef.current = [];
    inflightGovernanceExecutionRef.current = [];
    handledGovernanceActionIdsRef.current.clear();
    runtimeAssistantBufferRef.current = '';
  }, [managerSessionId, selectedAvatarId]);

  useEffect(() => {
    if (!managerProcessing) return;
    if (scheduledGovernanceExecutionRef.current.length === 0) return;
    const next = scheduledGovernanceExecutionRef.current.shift();
    if (next) {
      inflightGovernanceExecutionRef.current.push(next);
    }
  }, [managerProcessing]);

  useEffect(() => {
    if (selectedAvatarId) return;
    if (!selectedManagerGroupId) {
      setManagerSessionId(null);
      return;
    }
    try {
      const key = `${sessionStoragePrefix}__bootstrap:${selectedManagerGroupId}`;
      const saved = window.localStorage.getItem(key);
      setManagerSessionId(saved || null);
    } catch {
      setManagerSessionId(null);
    }
  }, [selectedManagerGroupId, selectedAvatarId, sessionStoragePrefix]);

  const loadPersistedRuntimeEvents = useCallback(async (
    sessionId: string,
    options?: {
      mode?: PersistedEventLoadMode;
      silent?: boolean;
    },
  ) => {
    const mode = options?.mode || 'latest';
    const silent = options?.silent ?? false;

    if (mode === 'older') {
      setPersistedEventsLoadingMore(true);
    } else if (!silent) {
      setPersistedEventsLoading(true);
    }
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
      } else if (mode === 'incremental') {
        const afterId = persistedLatestEventIdRef.current;
        if (!afterId || afterId <= 0) {
          return;
        }
        query.order = 'asc';
        query.afterEventId = afterId;
      }

      const fetched = await chatApi.listSessionEvents(sessionId, query);
      const normalized = mode === 'latest' || mode === 'older' ? fetched.slice().reverse() : fetched;
      const base = mode === 'latest' ? [] : persistedEventsRef.current;
      const merged = mode === 'latest'
        ? normalized
        : mergePersistedEvents(base, normalized, mode);

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
      if (!silent) addToast('error', msg);
    } finally {
      if (mode === 'older') {
        setPersistedEventsLoadingMore(false);
      } else if (!silent) {
        setPersistedEventsLoading(false);
      }
    }
  }, [addToast, t]);

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
    loadPersistedRuntimeEvents(managerSessionId, { mode: 'latest', silent: false });
  }, [loadPersistedRuntimeEvents, managerSessionId]);

  useEffect(() => {
    if (!managerProcessing || !managerSessionId) return;
    const timer = window.setInterval(() => {
      loadPersistedRuntimeEvents(managerSessionId, { mode: 'incremental', silent: true });
    }, 5000);
    return () => window.clearInterval(timer);
  }, [loadPersistedRuntimeEvents, managerProcessing, managerSessionId]);

  const persistGovernance = useCallback((
    updater: (current: AvatarGovernanceState) => AvatarGovernanceState,
    successMessage?: string,
  ) => {
    const targetAvatarId = selectedAvatarRef.current?.id;
    if (!targetAvatarId) {
      return Promise.resolve();
    }

    governancePersistQueueRef.current = governancePersistQueueRef.current
      .catch(() => undefined)
      .then(async () => {
        const avatar = selectedAvatarRef.current;
        if (!avatar || avatar.id !== targetAvatarId) return;

        const next = updater(governanceRef.current);
        governancePersistInFlightRef.current += 1;
        setSavingGovernance(true);
        try {
          const settings = mergeGovernanceSettings(
            avatar.settings as Record<string, unknown> | null | undefined,
            next,
          );
          const updated = await portalApi.update(teamId, avatar.id, { settings });
          setSelectedAvatar(updated);
          selectedAvatarRef.current = updated;
          setGovernance(next);
          governanceRef.current = next;
          if (successMessage) addToast('success', successMessage);
        } catch (err) {
          addToast('error', err instanceof Error ? err.message : t('common.error'));
        } finally {
          governancePersistInFlightRef.current = Math.max(
            0,
            governancePersistInFlightRef.current - 1,
          );
          if (governancePersistInFlightRef.current === 0) {
            setSavingGovernance(false);
          }
        }
      });
    return governancePersistQueueRef.current;
  }, [addToast, t, teamId]);

  const createManagerSession = useCallback(async (): Promise<string> => {
    if (selectedAvatar) {
      const managerAgentId =
        selectedAvatar.codingAgentId ||
        selectedAvatar.agentId ||
        selectedAvatar.serviceAgentId ||
        undefined;
      const res = await chatApi.createPortalManagerSession(teamId, managerAgentId);
      try {
        window.localStorage.setItem(`${sessionStoragePrefix}${selectedAvatar.id}`, res.session_id);
      } catch {}
      setManagerSessionId(res.session_id);
      return res.session_id;
    }
    const managerId = selectedManagerGroupId;
    if (!managerId) {
      throw new Error(t('digitalAvatar.states.noManagerAgent'));
    }
    const res = await chatApi.createPortalManagerSession(teamId, managerId);
    try {
      window.localStorage.setItem(`${sessionStoragePrefix}__bootstrap:${managerId}`, res.session_id);
    } catch {}
    setManagerSessionId(res.session_id);
    return res.session_id;
  }, [selectedAvatar, selectedManagerGroupId, sessionStoragePrefix, t, teamId]);

  const onManagerSessionCreated = useCallback((sessionId: string) => {
    setManagerSessionId(sessionId);
    const key = selectedAvatar
      ? `${sessionStoragePrefix}${selectedAvatar.id}`
      : (selectedManagerGroupId
        ? `${sessionStoragePrefix}__bootstrap:${selectedManagerGroupId}`
        : null);
    if (!key) return;
    try {
      window.localStorage.setItem(key, sessionId);
    } catch {}
  }, [selectedAvatar, selectedManagerGroupId, sessionStoragePrefix]);

  const sendManagerQuickPrompt = useCallback((kind:
    | 'createExternal'
    | 'createInternal'
    | 'audit'
    | 'optimize'
    | 'setAggressive'
    | 'setBalanced'
    | 'setConservative'
  ) => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const currentName = selectedAvatar?.name || t('digitalAvatar.workspace.currentAvatarFallback', '当前分身');
    const currentPortalId = selectedAvatar?.id || '';
    let text = '';
    if (kind === 'createExternal') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateExternal',
        '请为我创建一个新的对外数字分身：先明确服务对象与边界，再调用 create_digital_avatar 创建，并回读 profile 做校验，最后给我结果与风险清单。'
      )}\n管理Agent固定为: ${effectiveManagerAgentId}。要求新分身归属该管理Agent。`;
    } else if (kind === 'createInternal') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateInternal',
        '请为我创建一个新的对内数字分身：先定义任务目标与触发方式，再调用 create_digital_avatar 创建，并回读 profile 校验，最后输出上线建议。'
      )}\n管理Agent固定为: ${effectiveManagerAgentId}。要求新分身归属该管理Agent。`;
    } else if (kind === 'audit') {
      text = t(
        'digitalAvatar.workspace.quickPromptAudit',
        '请检查“{{name}}”当前能力边界（文档权限、扩展、技能、提示词），给出三项最小改进建议并标注风险。',
        { name: currentName }
      );
    } else if (kind === 'optimize') {
      text = t(
        'digitalAvatar.workspace.quickPromptOptimize',
        '请基于“{{name}}”最近运行情况，产出一份可执行优化工单（问题证据、修复方案、验证标准、回滚条件）。',
        { name: currentName }
      );
    } else {
      if (!selectedAvatar || !currentPortalId) {
        addToast('error', t('digitalAvatar.workspace.quickNeedAvatar', '请先在左侧选择一个分身，再设置治理阈值档位。'));
        return;
      }
      const threshold =
        kind === 'setAggressive' ? 3 : kind === 'setBalanced' ? 5 : 7;
      text = t(
        'digitalAvatar.workspace.quickPromptSetThreshold',
        '请把分身“{{name}}”(portal_id={{portalId}}) 的自动治理阈值设置为 {{count}}。请调用 portal_tools__configure_portal_service_agent 并通过 settings_patch 写入 digitalAvatarGovernanceConfig.autoProposalTriggerCount={{count}}，然后调用 portal_tools__get_portal_service_capability_profile 回读校验并汇报结果与风险。',
        {
          name: currentName,
          portalId: currentPortalId,
          count: threshold,
        }
      );
    }
    setManagerComposeRequest({
      id: makeId('quick_prompt'),
      text,
      autoSend: true,
    });
    addToast('success', t('digitalAvatar.actions.quickPromptSent', '已发送'));
  }, [addToast, effectiveManagerAgentId, selectedAvatar, t]);

  const copyGuideCommand = useCallback(async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      addToast('success', t('digitalAvatar.actions.copiedPrompt', '已复制'));
    } catch {
      addToast('error', t('common.error'));
    }
  }, [addToast, t]);

  const sendGuideCommandToManager = useCallback((text: string) => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    setManagerComposeRequest({
      id: makeId('guide_prompt'),
      text,
      autoSend: true,
    });
    setTab('workspace');
    addToast('success', t('digitalAvatar.actions.quickPromptSent', '已发送'));
  }, [addToast, effectiveManagerAgentId, t]);

  const dispatchGovernanceExecution = useCallback((
    text: string,
    binding?: Omit<GovernanceExecutionBinding, 'id'>,
  ) => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const prompt = text.trim();
    if (!prompt) return;
    const hasPendingExecution =
      managerProcessing ||
      inflightGovernanceExecutionRef.current.length > 0 ||
      scheduledGovernanceExecutionRef.current.length > 0;
    if (hasPendingExecution) {
      addToast(
        'warning',
        t(
          'digitalAvatar.governance.executionBusy',
          '管理 Agent 正在执行上一条治理任务，请等待完成后再提交下一项。'
        )
      );
      return;
    }
    const requestId = makeId('governance_exec');
    handledGovernanceActionIdsRef.current.delete(requestId);
    const structuredPrompt = `${prompt}\n\n请在最终回复末尾严格追加一个结构化回执（不要解释，不要省略）：\n<governance_action_result>{\"action_id\":\"${requestId}\",\"outcome\":\"success|partial|failed\",\"summary\":\"一句话结果摘要\",\"reason\":\"失败/部分成功原因；成功可留空\"}</governance_action_result>`;
    setManagerComposeRequest({
      id: requestId,
      text: structuredPrompt,
      autoSend: true,
    });
    if (binding) {
      scheduledGovernanceExecutionRef.current.push({
        ...binding,
        id: requestId,
      });
    }
    addToast('success', t('digitalAvatar.governance.executionQueued', '已提交管理 Agent 执行'));
  }, [addToast, effectiveManagerAgentId, managerProcessing, t]);

  const applyGovernanceBindingOutcome = useCallback((
    binding: GovernanceExecutionBinding,
    outcome: GovernanceActionReceipt['outcome'],
    reasonText?: string,
  ) => {
    const now = toIsoNow();
    const failed = outcome === 'failed';
    const partial = outcome === 'partial';
    const reason = (reasonText || '').trim();
    persistGovernance((current) => {
      if (binding.entityType === 'capability') {
        return {
          ...current,
          capabilityRequests: current.capabilityRequests.map((item) => {
            if (item.id !== binding.targetId) return item;
            if (!failed && !partial) {
              return {
                ...item,
                status: binding.targetStatus as CapabilityGapRequest['status'],
                decisionReason: reason || item.decisionReason,
                updatedAt: now,
              };
            }
            return {
              ...item,
              status: 'needs_human',
              decision: 'require_human_confirm',
              decisionReason: reason || item.decisionReason,
              updatedAt: now,
            };
          }),
        };
      }
      if (binding.entityType === 'proposal') {
        return {
          ...current,
          gapProposals: current.gapProposals.map((item) =>
            item.id !== binding.targetId
              ? item
              : {
                  ...item,
                  status: failed || partial ? 'pending_approval' : (binding.targetStatus as ProposalStatus),
                  updatedAt: now,
                }
          ),
        };
      }
      return {
        ...current,
        optimizationTickets: current.optimizationTickets.map((item) => {
          if (item.id !== binding.targetId) return item;
          if (!failed && !partial) {
            return {
              ...item,
              status: binding.targetStatus as OptimizationStatus,
              updatedAt: now,
            };
          }
          return {
            ...item,
            status: binding.targetStatus === 'deployed' ? 'rolled_back' : 'rejected',
            updatedAt: now,
          };
        }),
      };
    });
  }, [persistGovernance]);

  const takeGovernanceBindingByActionId = useCallback((actionId: string): GovernanceExecutionBinding | undefined => {
    const normalized = actionId.trim();
    if (!normalized) return undefined;
    const inflightIdx = inflightGovernanceExecutionRef.current.findIndex((item) => item.id === normalized);
    if (inflightIdx >= 0) {
      const [picked] = inflightGovernanceExecutionRef.current.splice(inflightIdx, 1);
      return picked;
    }
    const scheduledIdx = scheduledGovernanceExecutionRef.current.findIndex((item) => item.id === normalized);
    if (scheduledIdx >= 0) {
      const [picked] = scheduledGovernanceExecutionRef.current.splice(scheduledIdx, 1);
      return picked;
    }
    return undefined;
  }, []);

  const applyGovernanceReceipt = useCallback((receipt: GovernanceActionReceipt): boolean => {
    const actionId = receipt.actionId.trim();
    if (!actionId) return false;
    if (handledGovernanceActionIdsRef.current.has(actionId)) return true;
    const binding = takeGovernanceBindingByActionId(actionId);
    if (!binding) return false;

    handledGovernanceActionIdsRef.current.add(actionId);
    applyGovernanceBindingOutcome(binding, receipt.outcome, receipt.reason || receipt.summary);
    return true;
  }, [applyGovernanceBindingOutcome, takeGovernanceBindingByActionId]);

  const createAutoCapabilityRequest = useCallback((item: RuntimeSuggestion, nowIso: string): CapabilityGapRequest => ({
    id: makeId('gap'),
    title: item.title.trim(),
    detail: item.evidence.trim(),
    requestedScope: [`problem:${item.problemType}`],
    risk: item.risk,
    status: 'pending',
    source: 'avatar',
    createdAt: nowIso,
    updatedAt: nowIso,
  }), []);

  const createAutoOptimizationTicket = useCallback((item: RuntimeSuggestion, nowIso: string): OptimizationTicket => ({
    id: makeId('opt'),
    title: item.title.trim(),
    problemType: item.problemType,
    evidence: item.evidence.trim(),
    proposal: item.proposal.trim(),
    expectedGain: item.expectedGain.trim(),
    risk: item.risk,
    status: 'pending',
    createdAt: nowIso,
    updatedAt: nowIso,
  }), []);

  const maybeCreateAutoGapProposal = useCallback((
    current: AvatarGovernanceState,
    item: RuntimeSuggestion,
    nowIso: string,
  ): AgentGapProposal | null => {
    const scopeToken = `problem:${item.problemType}`;
    const relatedPendingRequestCount = current.capabilityRequests.filter(
      (request) =>
        request.status === 'pending' &&
        request.requestedScope.some((scope) => scope === scopeToken),
    ).length;
    if (relatedPendingRequestCount < automationConfig.autoProposalTriggerCount) {
      return null;
    }

    const proposalTitle = t(
      'digitalAvatar.governance.autoProposalTitle',
      '新增分身提案：{{problem}}能力长期缺口',
      { problem: item.problemType },
    );
    const hasOpenProposal = current.gapProposals.some(
      (proposal) =>
        proposal.title === proposalTitle &&
        proposal.status !== 'rejected',
    );
    if (hasOpenProposal) {
      return null;
    }

    return {
      id: makeId('proposal'),
      title: proposalTitle,
      description: t(
        'digitalAvatar.governance.autoProposalDescription',
        '近阶段该类型能力缺口多次重复出现，建议新增专用分身并进入人工审批。',
      ),
      expectedGain: t(
        'digitalAvatar.governance.autoProposalGain',
        '通过能力隔离减少重复提权与失败重试，提升交付稳定性。',
      ),
      status: 'pending_approval',
      proposedBy: 'manager',
      createdAt: nowIso,
      updatedAt: nowIso,
    };
  }, [automationConfig.autoProposalTriggerCount, t]);

  const handleRuntimeEvent = useCallback((event: ChatRuntimeEvent) => {
    if (event.kind === 'text') {
      const chunk = event.text || '';
      if (!chunk) return;
      const merged = `${runtimeAssistantBufferRef.current}${chunk}`;
      runtimeAssistantBufferRef.current = merged.length > 120000
        ? merged.slice(-120000)
        : merged;
      return;
    }

    if (event.kind === 'done') {
      const receipts = parseGovernanceActionReceipts(runtimeAssistantBufferRef.current);
      runtimeAssistantBufferRef.current = '';
      if (receipts.length > 0) {
        for (const receipt of receipts) {
          applyGovernanceReceipt(receipt);
        }
      } else {
        let binding: GovernanceExecutionBinding | undefined;
        while (inflightGovernanceExecutionRef.current.length > 0) {
          const candidate = inflightGovernanceExecutionRef.current.shift();
          if (!candidate) break;
          if (handledGovernanceActionIdsRef.current.has(candidate.id)) continue;
          binding = candidate;
          break;
        }
        if (binding) {
          handledGovernanceActionIdsRef.current.add(binding.id);
          const failed = isRuntimeDoneFailure(event.detail);
          const outcome: GovernanceActionReceipt['outcome'] = failed ? 'failed' : 'success';
          const reason = typeof event.detail?.error === 'string' ? event.detail.error : undefined;
          applyGovernanceBindingOutcome(binding, outcome, reason);
        }
      }
    }

    const suggestion = summarizeRuntimeFailure(event, runtimeSuggestionText);
    if (!suggestion) return;

    persistGovernance((current) => {
      const duplicateRuntime = current.runtimeLogs.some(
        (item) =>
          item.title === suggestion.title &&
          item.evidence === suggestion.evidence,
      );
      if (duplicateRuntime) return current;

      const nowIso = toIsoNow();
      const hasSamePendingRequest = current.capabilityRequests.some(
        (request) =>
          request.status === 'pending' &&
          request.title === suggestion.title &&
          request.detail === suggestion.evidence,
      );
      const hasSamePendingTicket = current.optimizationTickets.some(
        (ticket) =>
          ticket.status === 'pending' &&
          ticket.title === suggestion.title &&
          ticket.problemType === suggestion.problemType &&
          ticket.evidence === suggestion.evidence,
      );

      let capabilityRequests = current.capabilityRequests;
      let optimizationTickets = current.optimizationTickets;
      let gapProposals = current.gapProposals;
      let runtimeStatus: RuntimeLogStatus = 'pending';

      if (!hasSamePendingRequest) {
        capabilityRequests = [createAutoCapabilityRequest(suggestion, nowIso), ...capabilityRequests];
        runtimeStatus = 'requested';
      }
      if (!hasSamePendingTicket) {
        optimizationTickets = [createAutoOptimizationTicket(suggestion, nowIso), ...optimizationTickets];
        if (runtimeStatus === 'pending') runtimeStatus = 'ticketed';
      }

      const autoProposal = maybeCreateAutoGapProposal(
        { ...current, capabilityRequests, optimizationTickets, gapProposals },
        suggestion,
        nowIso,
      );
      if (autoProposal) {
        gapProposals = [autoProposal, ...gapProposals];
      }

      return {
        ...current,
        capabilityRequests,
        optimizationTickets,
        gapProposals,
        runtimeLogs: [{ ...suggestion, status: runtimeStatus }, ...current.runtimeLogs]
          .slice(0, MAX_RUNTIME_LOGS),
      };
    });
  }, [
    applyGovernanceBindingOutcome,
    applyGovernanceReceipt,
    createAutoCapabilityRequest,
    createAutoOptimizationTicket,
    maybeCreateAutoGapProposal,
    persistGovernance,
    runtimeSuggestionText,
  ]);

  const updateRuntimeSuggestionStatus = useCallback((id: string, status: RuntimeLogStatus) => {
    persistGovernance((current) => ({
      ...current,
      runtimeLogs: current.runtimeLogs.map((item) => (item.id === id ? { ...item, status } : item)),
    }));
  }, [persistGovernance]);

  const dismissRuntimeSuggestion = useCallback((id: string) => {
    updateRuntimeSuggestionStatus(id, 'dismissed');
  }, [updateRuntimeSuggestionStatus]);

  const resetRuntimeSuggestion = useCallback((id: string) => {
    updateRuntimeSuggestionStatus(id, 'pending');
  }, [updateRuntimeSuggestionStatus]);

  const decideCapabilityRequest = useCallback((id: string, decision: DecisionMode, reason?: string) => {
    const now = toIsoNow();
    const item = governanceRef.current.capabilityRequests.find((it) => it.id === id);
    const portalId = selectedAvatarRef.current?.id;
    persistGovernance(
      current => ({
        ...current,
        capabilityRequests: current.capabilityRequests.map((item) =>
          item.id !== id
            ? item
            : {
                ...item,
                status: toDecisionStatus(decision),
                decision,
                decisionReason: reason || item.decisionReason,
                updatedAt: now,
              }
        ),
      }),
      t('common.saved')
    );
    if (
      item &&
      portalId &&
      (decision === 'approve_direct' || decision === 'approve_sandbox')
    ) {
      const mode = decision === 'approve_sandbox' ? 'sandbox' : 'direct';
      const executionPrompt = t(
        'digitalAvatar.governance.capabilityExecutionPrompt',
        '请执行能力缺口请求并完成回读校验。portal_id={{portalId}}，模式={{mode}}。\n请求标题：{{title}}\n请求说明：{{detail}}\n要求：\n1) 优先调用 portal_tools__configure_portal_service_agent 完成最小权限变更；\n2) 必须调用 portal_tools__get_portal_service_capability_profile 回读验证；\n3) 输出变更摘要、风险与回滚建议。',
        {
          portalId,
          mode,
          title: item.title,
          detail: item.detail,
        }
      );
      dispatchGovernanceExecution(executionPrompt, {
        entityType: 'capability',
        targetId: item.id,
        targetStatus: toDecisionStatus(decision),
      });
    }
  }, [dispatchGovernanceExecution, persistGovernance, t]);

  const updateGapProposalStatus = useCallback((id: string, status: ProposalStatus) => {
    const now = toIsoNow();
    const item = governanceRef.current.gapProposals.find((it) => it.id === id);
    persistGovernance(
      current => ({
        ...current,
        gapProposals: current.gapProposals.map((item) =>
          item.id === id ? { ...item, status, updatedAt: now } : item
        ),
      }),
      t('common.saved')
    );
    if (item && ['approved', 'pilot', 'active'].includes(status)) {
      const executionPrompt = t(
        'digitalAvatar.governance.proposalExecutionPrompt',
        '请根据已通过提案进入执行闭环：\n提案：{{title}}\n说明：{{desc}}\n目标状态：{{status}}\n要求：产出执行计划（能力/权限/文档范围）、实施步骤、验证标准与回滚策略。',
        {
          title: item.title,
          desc: item.description,
          status,
        }
      );
      dispatchGovernanceExecution(executionPrompt, {
        entityType: 'proposal',
        targetId: item.id,
        targetStatus: status,
      });
    }
  }, [dispatchGovernanceExecution, persistGovernance, t]);

  const updateOptimizationStatus = useCallback((id: string, status: OptimizationStatus) => {
    const now = toIsoNow();
    const item = governanceRef.current.optimizationTickets.find((it) => it.id === id);
    persistGovernance(
      current => ({
        ...current,
        optimizationTickets: current.optimizationTickets.map((item) =>
          item.id === id ? { ...item, status, updatedAt: now } : item
        ),
      }),
      t('common.saved')
    );
    if (item && ['approved', 'experimenting', 'deployed'].includes(status)) {
      const executionPrompt = t(
        'digitalAvatar.governance.ticketExecutionPrompt',
        '请执行优化工单并回传结果：\n工单：{{title}}\n问题类型：{{problemType}}\n证据：{{evidence}}\n方案：{{proposal}}\n目标状态：{{status}}\n要求：执行后提供验证结果、风险变化与是否继续推进建议。',
        {
          title: item.title,
          problemType: item.problemType,
          evidence: item.evidence,
          proposal: item.proposal,
          status,
        }
      );
      dispatchGovernanceExecution(executionPrompt, {
        entityType: 'ticket',
        targetId: item.id,
        targetStatus: status,
      });
    }
  }, [dispatchGovernanceExecution, persistGovernance, t]);

  const clearRuntimeSuggestions = () => {
    persistGovernance((current) => ({ ...current, runtimeLogs: [] }));
  };

  const refreshPersistedEvents = useCallback(() => {
    if (!managerSessionId) {
      addToast('error', t('digitalAvatar.governance.runtimeEventsNoSession', '暂无可追溯会话，请先与管理 Agent 开始对话。'));
      return;
    }
    loadPersistedRuntimeEvents(managerSessionId, { mode: 'latest', silent: false });
  }, [addToast, loadPersistedRuntimeEvents, managerSessionId, t]);

  const loadOlderPersistedEvents = useCallback(() => {
    if (!managerSessionId) {
      addToast('error', t('digitalAvatar.governance.runtimeEventsNoSession', '暂无可追溯会话，请先与管理 Agent 开始对话。'));
      return;
    }
    if (!persistedEventsHasMore || persistedEventsLoadingMore) return;
    loadPersistedRuntimeEvents(managerSessionId, { mode: 'older', silent: false });
  }, [
    addToast,
    loadPersistedRuntimeEvents,
    managerSessionId,
    persistedEventsHasMore,
    persistedEventsLoadingMore,
    t,
  ]);

  const saveAutomationConfig = useCallback(async () => {
    if (!selectedAvatar || !canManage) return;
    const nextValue = Math.min(10, Math.max(1, Math.round(autoProposalTriggerCountDraft || 3)));
    setSavingAutomationConfig(true);
    try {
      const settings = mergeGovernanceAutomationConfig(
        selectedAvatar.settings as Record<string, unknown> | null | undefined,
        { autoProposalTriggerCount: nextValue },
      );
      const updated = await portalApi.update(teamId, selectedAvatar.id, { settings });
      setSelectedAvatar(updated);
      setAutoProposalTriggerCountDraft(nextValue);
      addToast('success', t('common.saved'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSavingAutomationConfig(false);
    }
  }, [
    addToast,
    autoProposalTriggerCountDraft,
    canManage,
    selectedAvatar,
    t,
    teamId,
  ]);

  const openLaboratory = () => {
    navigate(`/admin/teams/${teamId}?section=laboratory`);
  };

  const governanceStats = useMemo(() => {
    const pendingCapability = governance.capabilityRequests.filter(x => x.status === 'pending').length;
    const pendingProposals = governance.gapProposals.filter(x => x.status === 'pending_approval').length;
    const pendingTickets = governance.optimizationTickets.filter(x => x.status === 'pending').length;
    return { pendingCapability, pendingProposals, pendingTickets };
  }, [governance]);

  return (
    <div className="h-[calc(100vh-40px)] flex flex-col p-3 sm:p-4 gap-3 overflow-hidden">
      <div className="rounded-xl border bg-card p-3 sm:p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2.5 min-w-0">
            <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary shrink-0">
              <UserRound className="h-4.5 w-4.5" />
            </div>
            <div className="min-w-0">
              <h2 className="text-sm font-semibold truncate">{t('digitalAvatar.title')}</h2>
              <p className="text-caption text-muted-foreground truncate">{t('digitalAvatar.description')}</p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={() => setTab(tab === 'workspace' ? 'guide' : 'workspace')}>
              {tab === 'workspace' ? t('digitalAvatar.tabs.guide') : t('digitalAvatar.tabs.workspace')}
            </Button>
            <Button variant="outline" size="sm" onClick={() => loadAvatars(false)} disabled={refreshing}>
              {refreshing ? <Loader2 className="w-3.5 h-3.5 mr-1 animate-spin" /> : <RefreshCw className="w-3.5 h-3.5 mr-1" />}
              {t('digitalAvatar.actions.refresh')}
            </Button>
            {canManage && (
              <>
                <Button variant="outline" size="sm" onClick={() => setCreateManagerOpen(true)}>
                  <Users className="w-3.5 h-3.5 mr-1" />
                  {t('digitalAvatar.actions.createManager', '新建管理 Agent')}
                </Button>
                <Button
                  size="sm"
                  onClick={() => {
                    if (!selectedManagerGroupId) {
                      addToast('error', t('digitalAvatar.states.noManagerAgent'));
                      return;
                    }
                    setCreateOpen(true);
                  }}
                >
                  <Plus className="w-3.5 h-3.5 mr-1" />
                  {t('digitalAvatar.actions.create')}
                </Button>
              </>
            )}
          </div>
        </div>
      </div>

      {tab === 'guide' ? (
        <div className="min-h-0 flex-1 overflow-y-auto rounded-xl border bg-card">
          <DigitalAvatarGuide
            canSendCommand={Boolean(effectiveManagerAgentId)}
            onCopyCommand={copyGuideCommand}
            onSendCommand={sendGuideCommandToManager}
          />
        </div>
      ) : (
        <div className="min-h-0 flex-1 grid grid-cols-1 lg:grid-cols-[260px_minmax(0,1fr)_360px] gap-3">
          <Card className="min-h-0 flex flex-col">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm flex items-center justify-between">
                <span>{t('digitalAvatar.list.title')}</span>
                <span className="text-caption font-normal text-muted-foreground">{visibleAvatars.length}</span>
              </CardTitle>
              <div className="space-y-1.5">
                <p className="text-[11px] text-muted-foreground">
                  {t('digitalAvatar.labels.managerGroup', '管理 Agent 组')}
                </p>
                {managerGroupOptions.length === 0 ? (
                  <div className="space-y-2">
                    <div className="rounded-md border border-dashed px-2 py-2 text-[11px] text-muted-foreground">
                      {t('digitalAvatar.states.noManagerAgent')}
                    </div>
                    {canManage && (
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        className="h-7 w-full text-[11px]"
                        onClick={() => setCreateManagerOpen(true)}
                      >
                        <Users className="w-3 h-3 mr-1" />
                        {t('digitalAvatar.actions.createManager', '新建管理 Agent')}
                      </Button>
                    )}
                  </div>
                ) : (
                  <>
                    <select
                      className="h-8 w-full rounded-md border bg-background px-2 text-xs"
                      value={selectedManagerGroupId || ''}
                      onChange={(e) => {
                        setSelectedAvatarId(null);
                        setBootstrapManagerAgentId(e.target.value);
                      }}
                    >
                      {managerGroupOptions.map((agent) => (
                        <option key={agent.id} value={agent.id}>
                          {agent.name}
                        </option>
                      ))}
                    </select>
                    <div className="rounded-md border bg-muted/20 px-2 py-1.5 text-[10px] text-muted-foreground">
                      {t('digitalAvatar.list.groupStats', '分组统计')}:
                      {' '}
                      {t('digitalAvatar.filters.all')} {managerGroupStats.total}
                      {' · '}
                      {t('digitalAvatar.filters.external')} {managerGroupStats.external}
                      {' · '}
                      {t('digitalAvatar.filters.internal')} {managerGroupStats.internal}
                    </div>
                  </>
                )}
              </div>
              <div className="flex items-center gap-1">
                {(['all', 'external', 'internal'] as AvatarFilter[]).map(type => (
                  <button
                    key={type}
                    className={`px-2 py-1 text-caption rounded border ${filter === type ? 'bg-primary/10 border-primary/50 text-primary' : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'}`}
                    onClick={() => setFilter(type)}
                  >
                    {type === 'all'
                      ? t('digitalAvatar.filters.all')
                      : type === 'external'
                      ? t('digitalAvatar.filters.external')
                      : t('digitalAvatar.filters.internal')}
                  </button>
                ))}
              </div>
            </CardHeader>
            <CardContent className="min-h-0 flex-1 overflow-y-auto space-y-2">
              {loading ? (
                <div className="h-28 flex items-center justify-center text-muted-foreground text-caption">
                  <Loader2 className="w-4 h-4 animate-spin mr-1.5" />
                  {t('digitalAvatar.states.loading')}
                </div>
              ) : visibleAvatars.length === 0 ? (
                <div className="rounded-lg border border-dashed p-3 text-caption text-muted-foreground">
                  <p className="font-medium text-foreground">{t('digitalAvatar.states.noAvatars')}</p>
                  <p className="mt-1">{t('digitalAvatar.states.noAvatarsHint')}</p>
                </div>
              ) : (
                visibleAvatars.map(avatar => {
                  const selected = avatar.id === selectedAvatarId;
                  const avatarType = detectAvatarType(avatar);
                  return (
                    <button
                      key={avatar.id}
                      className={`w-full text-left rounded-md border px-2.5 py-2 transition-colors ${selected ? 'border-primary/60 bg-primary/8' : 'border-border/60 hover:border-primary/40'}`}
                      onClick={() => setSelectedAvatarId(avatar.id)}
                    >
                      <p className="text-xs font-medium truncate">{avatar.name}</p>
                      <div className="mt-1 flex items-center justify-between gap-2 text-caption text-muted-foreground">
                        <span className="truncate">/p/{avatar.slug}</span>
                        <span className="shrink-0">
                          {avatarType === 'external'
                            ? t('digitalAvatar.types.external')
                            : avatarType === 'internal'
                            ? t('digitalAvatar.types.internal')
                            : t('digitalAvatar.labels.unset')}
                        </span>
                      </div>
                    </button>
                  );
                })
              )}
            </CardContent>
          </Card>

          <Card className="min-h-0 flex flex-col">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">
                {selectedAvatar
                  ? selectedAvatar.name
                  : t('digitalAvatar.workspace.managerBootstrapTitle', '数字分身创建工作台')}
              </CardTitle>
              <p className="text-caption text-muted-foreground">
                {t('digitalAvatar.workspace.managerConsoleHint')}
              </p>
            </CardHeader>
            <CardContent className="min-h-0 flex-1 overflow-hidden">
              {!effectiveManagerAgentId ? (
                <div className="h-full flex items-center justify-center">
                  <div className="text-center text-caption text-muted-foreground space-y-2">
                    <p>{t('digitalAvatar.states.noManagerAgent')}</p>
                    {canManage && (
                      <Button size="sm" variant="outline" onClick={openLaboratory}>
                        <ExternalLink className="w-3.5 h-3.5 mr-1" />
                        {t('digitalAvatar.actions.openLaboratory')}
                      </Button>
                    )}
                  </div>
                </div>
              ) : (
                <div className="h-full flex flex-col gap-2">
                  {!selectedAvatar && (
                    <div className="rounded-md border border-primary/20 bg-primary/5 p-2.5 text-caption text-muted-foreground space-y-2">
                      <p className="text-foreground font-medium">
                        {t('digitalAvatar.workspace.bootstrapHintTitle', '先与管理 Agent 对话，再创建分身')}
                      </p>
                      <p>
                        {t(
                          'digitalAvatar.workspace.bootstrapHintBody',
                          '管理 Agent 会先确认目标与能力边界，再调用工具创建并配置数字分身。'
                        )}
                      </p>
                      <div className="flex items-center gap-2">
                        <span className="shrink-0 text-[11px] text-muted-foreground">
                          {t('digitalAvatar.labels.managerAgent')}
                        </span>
                        <select
                          className="h-8 w-full rounded-md border bg-background px-2 text-xs"
                          value={effectiveManagerAgentId}
                          onChange={(e) => setBootstrapManagerAgentId(e.target.value)}
                        >
                          {managerGroupOptions.map((agent) => (
                            <option key={agent.id} value={agent.id}>
                              {agent.name}
                            </option>
                          ))}
                        </select>
                      </div>
                    </div>
                  )}
                  <div className="rounded-md border border-border/70 bg-background p-2.5 space-y-2">
                    <div className="flex items-center justify-between gap-2">
                      <p className="text-xs font-medium text-foreground">
                        {t('digitalAvatar.workspace.quickPromptsTitle', '管理对话快捷指令')}
                      </p>
                      <span className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.workspace.quickPromptsHint', '点击后自动发送给管理 Agent')}
                      </span>
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('createExternal')}
                      >
                        {t('digitalAvatar.workspace.quickCreateExternal', '创建对外分身')}
                      </button>
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('createInternal')}
                      >
                        {t('digitalAvatar.workspace.quickCreateInternal', '创建对内分身')}
                      </button>
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('audit')}
                      >
                        {t('digitalAvatar.workspace.quickAuditCurrent', '审查当前能力')}
                      </button>
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('optimize')}
                      >
                        {t('digitalAvatar.workspace.quickOptimizeCurrent', '生成优化工单')}
                      </button>
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('setAggressive')}
                      >
                        {t('digitalAvatar.workspace.quickSetAggressive', '阈值激进(3)')}
                      </button>
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('setBalanced')}
                      >
                        {t('digitalAvatar.workspace.quickSetBalanced', '阈值平衡(5)')}
                      </button>
                      <button
                        className="px-2 py-1 text-[11px] rounded border border-border/70 hover:bg-muted"
                        onClick={() => sendManagerQuickPrompt('setConservative')}
                      >
                        {t('digitalAvatar.workspace.quickSetConservative', '阈值保守(7)')}
                      </button>
                    </div>
                  </div>
                  <div className="min-h-0 flex-1 overflow-hidden">
                    <ChatConversation
                      sessionId={managerSessionId}
                      agentId={effectiveManagerAgentId}
                      agentName={managerAgent?.name || t('digitalAvatar.labels.managerAgent')}
                      agent={managerAgent || undefined}
                      teamId={teamId}
                      createSession={createManagerSession}
                      onSessionCreated={onManagerSessionCreated}
                      onRuntimeEvent={handleRuntimeEvent}
                      onProcessingChange={setManagerProcessing}
                      composeRequest={managerComposeRequest}
                    />
                  </div>
                </div>
              )}
            </CardContent>
          </Card>

          <div className="min-h-0 overflow-y-auto space-y-3">
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <ShieldAlert className="w-4 h-4" />
                  {t('digitalAvatar.workspace.protocolTitle')}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-1 text-caption text-muted-foreground">
                <p>{t('digitalAvatar.workspace.protocolStep1')}</p>
                <p>{t('digitalAvatar.workspace.protocolStep2')}</p>
                <p>{t('digitalAvatar.workspace.protocolStep3')}</p>
                <p>{t('digitalAvatar.workspace.protocolStep4')}</p>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between">
                  <span className="flex items-center gap-1.5">
                    <Bot className="w-4 h-4" />
                    {t('digitalAvatar.workspace.capabilityTitle')}
                  </span>
                  {savingGovernance && <Loader2 className="w-3.5 h-3.5 animate-spin text-muted-foreground" />}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-caption">
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.managerAgent')}</p>
                  <p className="mt-0.5 text-xs font-medium">{getAgentName(agents, effectiveManagerAgentId, t('digitalAvatar.labels.unset'))}</p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.serviceAgent')}</p>
                  <p className="mt-0.5 text-xs font-medium">{getAgentName(agents, serviceAgentId, t('digitalAvatar.labels.unset'))}</p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.documentAccess')}</p>
                  <p className="mt-0.5 text-xs font-medium">{selectedAvatar?.documentAccessMode || t('digitalAvatar.labels.unset')}</p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.allowedExtensions')}</p>
                  <p className="mt-0.5 text-xs font-medium">
                    {(selectedAvatar?.allowedExtensions || []).length > 0
                      ? selectedAvatar?.allowedExtensions?.join(', ')
                      : t('digitalAvatar.labels.unset')}
                  </p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.allowedSkills')}</p>
                  <p className="mt-0.5 text-xs font-medium">
                    {(selectedAvatar?.allowedSkillIds || []).length > 0
                      ? selectedAvatar?.allowedSkillIds?.join(', ')
                      : t('digitalAvatar.labels.unset')}
                  </p>
                </div>
                <Button variant="outline" size="sm" className="w-full" onClick={openLaboratory}>
                  <ExternalLink className="w-3.5 h-3.5 mr-1" />
                  {t('digitalAvatar.actions.openLaboratory')}
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <Clock3 className="w-4 h-4" />
                  {t('digitalAvatar.governance.capabilityRequestTitle', '能力缺口请求（管理者决策）')}
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t('digitalAvatar.governance.capabilityRequestHint', '分身遇到权限或工具不足时，先提交请求，由管理 Agent 决定执行路径。')}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="rounded-md border bg-muted/20 p-2 text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.autoGeneratedHint',
                    '该列表由管理 Agent 根据运行日志自动生成；此处仅保留人工审批。'
                  )}
                </div>
                <div className="space-y-2 max-h-[230px] overflow-y-auto pr-1">
                  {governance.capabilityRequests.length === 0 ? (
                    <p className="text-caption text-muted-foreground">{t('digitalAvatar.governance.noRequest', '暂无提权请求')}</p>
                  ) : governance.capabilityRequests.map((item) => (
                    <div key={item.id} className="rounded-md border p-2">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs font-medium truncate">{item.title}</p>
                        <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                          {t(`digitalAvatar.governance.status.${item.status}`, item.status)}
                        </span>
                      </div>
                      {item.detail && <p className="mt-1 text-caption text-muted-foreground line-clamp-3">{item.detail}</p>}
                      <div className="mt-1.5 flex flex-wrap items-center gap-1">
                        <span className="text-[10px] text-muted-foreground">{item.risk}</span>
                        <span className="text-[10px] text-muted-foreground">·</span>
                        <span className="text-[10px] text-muted-foreground">{item.source}</span>
                      </div>
                      {canManage && item.status === 'pending' && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.id, 'approve_direct')}>
                            {t('digitalAvatar.governance.action.approve', '通过')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.id, 'approve_sandbox')}>
                            {t('digitalAvatar.governance.action.sandbox', '沙箱通过')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.id, 'require_human_confirm')}>
                            {t('digitalAvatar.governance.action.human', '转人工')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.id, 'deny')}>
                            {t('digitalAvatar.governance.action.deny', '拒绝')}
                          </button>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <Sparkles className="w-4 h-4" />
                  {t('digitalAvatar.governance.gapProposalTitle', '缺口提案（新增分身建议）')}
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t('digitalAvatar.governance.gapProposalHint', '管理 Agent 发现长期缺口后，形成提案并进入人工审批。')}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="rounded-md border bg-muted/20 p-2 text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.autoProposalHint',
                    '系统会在同类能力缺口持续出现时自动生成新增分身提案，并进入人工审批。'
                  )}
                </div>
                <div className="space-y-2 max-h-[220px] overflow-y-auto pr-1">
                  {governance.gapProposals.length === 0 ? (
                    <p className="text-caption text-muted-foreground">{t('digitalAvatar.governance.noProposal', '暂无缺口提案')}</p>
                  ) : governance.gapProposals.map((item) => (
                    <div key={item.id} className="rounded-md border p-2">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs font-medium truncate">{item.title}</p>
                        <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                          {t(`digitalAvatar.governance.proposalStatus.${toProposalStatusLabel(item.status)}`, item.status)}
                        </span>
                      </div>
                      {item.description && <p className="mt-1 text-caption text-muted-foreground line-clamp-3">{item.description}</p>}
                      {item.expectedGain && (
                        <p className="mt-1 text-[10px] text-muted-foreground">
                          {t('digitalAvatar.governance.gainLabel', '预期收益')}: {item.expectedGain}
                        </p>
                      )}
                      {canManage && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.id, 'pending_approval')}>
                            {t('digitalAvatar.governance.action.pending', '待审批')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.id, 'approved')}>
                            {t('digitalAvatar.governance.action.approve', '通过')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.id, 'pilot')}>
                            {t('digitalAvatar.governance.action.pilot', '试运行')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.id, 'active')}>
                            {t('digitalAvatar.governance.action.active', '生效')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.id, 'rejected')}>
                            {t('digitalAvatar.governance.action.reject', '拒绝')}
                          </button>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <Clock3 className="w-4 h-4" />
                    {t('digitalAvatar.governance.runtimeLogTitle', '运行日志建议')}
                  </span>
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.pendingCount', '待处理')} {runtimePendingCount}/{governance.runtimeLogs.length}
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
                      onClick={clearRuntimeSuggestions}
                      disabled={governance.runtimeLogs.length === 0}
                    >
                      {t('digitalAvatar.governance.clearRuntimeLog', '清空')}
                    </Button>
                  </div>
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t('digitalAvatar.governance.runtimeLogHint', '汇总管理 Agent 运行中的关键失败事件，可一键转治理动作。')}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="flex items-center gap-1">
                  <button
                    className={`px-2 py-1 text-caption rounded border ${runtimeLogFilter === 'pending' ? 'bg-primary/10 border-primary/50 text-primary' : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'}`}
                    onClick={() => setRuntimeLogFilter('pending')}
                  >
                    {t('digitalAvatar.governance.runtimeFilterPending', '仅看未处理')}
                  </button>
                  <button
                    className={`px-2 py-1 text-caption rounded border ${runtimeLogFilter === 'all' ? 'bg-primary/10 border-primary/50 text-primary' : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'}`}
                    onClick={() => setRuntimeLogFilter('all')}
                  >
                    {t('digitalAvatar.governance.runtimeFilterAll', '全部')}
                  </button>
                </div>
                <div className="space-y-2 max-h-[220px] overflow-y-auto pr-1">
                  {visibleRuntimeSuggestions.length === 0 ? (
                    <p className="text-caption text-muted-foreground">
                      {runtimeLogFilter === 'pending'
                        ? t('digitalAvatar.governance.noRuntimePending', '暂无待处理运行异常')
                        : t('digitalAvatar.governance.noRuntimeLog', '暂无运行异常日志')}
                    </p>
                  ) : visibleRuntimeSuggestions.map((item) => (
                    <div key={item.id} className="rounded-md border border-status-warning/35 bg-status-warning/10 p-2">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs font-medium truncate">{item.title}</p>
                        <div className="flex items-center gap-1">
                          <span className="text-[10px] text-muted-foreground">{item.problemType}</span>
                          <span className={`px-1.5 py-0.5 rounded text-[10px] border ${runtimeStatusClass(item.status)}`}>
                            {t(`digitalAvatar.governance.runtimeStatus.${item.status}`, item.status)}
                          </span>
                        </div>
                      </div>
                      <p className="mt-1 text-caption text-muted-foreground line-clamp-3">{item.evidence}</p>
                      <div className="mt-1.5 flex items-center justify-between gap-2">
                        <span className="text-[10px] text-muted-foreground">{new Date(item.createdAt).toLocaleString()}</span>
                        {item.status === 'pending' ? (
                          <div className="flex flex-wrap gap-1">
                            <button
                              className="px-1.5 py-1 text-[10px] rounded border hover:bg-background"
                              onClick={() => dismissRuntimeSuggestion(item.id)}
                            >
                              {t('common.dismiss', '忽略')}
                            </button>
                          </div>
                        ) : (
                          <button
                            className="px-1.5 py-1 text-[10px] rounded border hover:bg-background"
                            onClick={() => resetRuntimeSuggestion(item.id)}
                          >
                            {t('digitalAvatar.governance.runtimeRestore', '恢复待处理')}
                          </button>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <Clock3 className="w-4 h-4" />
                    {t('digitalAvatar.governance.runtimeEventsTitle', '完整运行日志（可追溯）')}
                  </span>
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.runtimeEventsCount', '事件 {{count}}', {
                        count: persistedEvents.length,
                      })}
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
                      onClick={loadOlderPersistedEvents}
                      disabled={!persistedEventsHasMore || persistedEventsLoadingMore || persistedEventsLoading}
                    >
                      {persistedEventsLoadingMore ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        t('digitalAvatar.governance.runtimeEventsLoadOlder', '加载更早')
                      )}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
                      onClick={refreshPersistedEvents}
                      disabled={persistedEventsLoading}
                    >
                      {persistedEventsLoading ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        <RefreshCw className="w-3 h-3" />
                      )}
                    </Button>
                  </div>
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.runtimeEventsHint',
                    '记录管理 Agent 全量执行事件（状态/思考/工具/结果/完成），支持分类筛选与追溯排查。'
                  )}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="flex flex-wrap items-center gap-1">
                  {(['all', 'error', 'tool', 'thinking', 'status'] as PersistedEventFilter[]).map((kind) => (
                    <button
                      key={kind}
                      className={`px-2 py-1 text-caption rounded border ${
                        persistedEventFilter === kind
                          ? 'bg-primary/10 border-primary/50 text-primary'
                          : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'
                      }`}
                      onClick={() => setPersistedEventFilter(kind)}
                    >
                      {t(`digitalAvatar.governance.runtimeEventsFilter.${kind}`, kind)}
                    </button>
                  ))}
                  <input
                    className="h-7 min-w-[120px] flex-1 rounded border bg-background px-2 text-[11px]"
                    placeholder={t('digitalAvatar.governance.runtimeEventsSearch', '搜索事件内容')}
                    value={persistedEventSearch}
                    onChange={(e) => setPersistedEventSearch(e.target.value)}
                  />
                </div>
                {persistedEventsError && (
                  <div className="rounded border border-status-error/40 bg-status-error/10 px-2 py-1 text-[10px] text-status-error-text">
                    {persistedEventsError}
                  </div>
                )}
                <div className="space-y-2 max-h-[260px] overflow-y-auto pr-1">
                  {visiblePersistedEvents.length === 0 ? (
                    <p className="text-caption text-muted-foreground">
                      {t('digitalAvatar.governance.runtimeEventsEmpty', '暂无可展示事件')}
                    </p>
                  ) : visiblePersistedEvents.map((event) => {
                    const severity = eventSeverity(event);
                    return (
                      <div key={`${event.run_id || 'no_run'}:${event.event_id}:${event.created_at}`} className={`rounded-md border p-2 ${severityClass(severity)}`}>
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0">
                            <p className="text-[11px] font-medium truncate">
                              #{event.event_id} · {event.event_type}
                            </p>
                            <p className="text-[10px] text-muted-foreground truncate">
                              {event.run_id || 'run:unknown'} · {new Date(event.created_at).toLocaleString()}
                            </p>
                          </div>
                          <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(severity === 'error' ? 'rejected' : severity === 'warn' ? 'pending' : 'approved')}`}>
                            {eventTypeBadge(event.event_type)}
                          </span>
                        </div>
                        <p className="mt-1 text-caption text-muted-foreground whitespace-pre-wrap break-words">
                          {eventSummary(event) || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}
                        </p>
                      </div>
                    );
                  })}
                </div>
                {!persistedEventsHasMore && persistedEvents.length > 0 && (
                  <p className="text-[10px] text-muted-foreground text-center">
                    {t('digitalAvatar.governance.runtimeEventsNoOlder', '已加载到最早事件')}
                  </p>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <ShieldAlert className="w-4 h-4" />
                    {t('digitalAvatar.governance.decisionAuditTitle', '治理决策审计')}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    {t('digitalAvatar.governance.decisionAuditCount', '记录 {{count}}', {
                      count: governanceAuditRows.length,
                    })}
                  </span>
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t('digitalAvatar.governance.decisionAuditHint', '集中展示管理 Agent 与人工的审批/驳回/部署等决策记录。')}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="space-y-2 max-h-[220px] overflow-y-auto pr-1">
                  {governanceAuditRows.length === 0 ? (
                    <p className="text-caption text-muted-foreground">
                      {t('digitalAvatar.governance.decisionAuditEmpty', '暂无决策记录')}
                    </p>
                  ) : governanceAuditRows.map((item) => (
                    <div key={item.id} className="rounded-md border p-2 bg-muted/20">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs font-medium truncate">{item.title}</p>
                        <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                          {item.type}
                        </span>
                      </div>
                      <p className="mt-1 text-caption text-muted-foreground line-clamp-2">
                        {item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}
                      </p>
                      <div className="mt-1 flex items-center justify-between gap-2">
                        <span className="text-[10px] text-muted-foreground">
                          {new Date(item.ts).toLocaleString()}
                        </span>
                        <span className="text-[10px] text-muted-foreground">{item.status}</span>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <Brain className="w-4 h-4" />
                  {t('digitalAvatar.workspace.optimizationTitle')}
                </CardTitle>
                <p className="text-caption text-muted-foreground">{t('digitalAvatar.workspace.optimizationHint')}</p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="rounded-md border bg-muted/20 p-2 space-y-1 text-caption text-muted-foreground">
                  <p>{t('digitalAvatar.workspace.optimizationSelf')}</p>
                  <p>{t('digitalAvatar.workspace.optimizationSupervisor')}</p>
                  <p>{t('digitalAvatar.workspace.optimizationProposal')}</p>
                </div>
                <div className="rounded-md border bg-muted/20 p-2 text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.autoTicketHint',
                    '优化工单由运行事件自动生成，人工只需审批/实验/部署。'
                  )}
                </div>

                <div className="space-y-2 max-h-[220px] overflow-y-auto pr-1">
                  {governance.optimizationTickets.length === 0 ? (
                    <p className="text-caption text-muted-foreground">{t('digitalAvatar.governance.noTicket', '暂无优化工单')}</p>
                  ) : governance.optimizationTickets.map((item) => (
                    <div key={item.id} className="rounded-md border p-2">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs font-medium truncate">{item.title}</p>
                        <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                          {t(`digitalAvatar.governance.ticketStatus.${toOptimizationStatusLabel(item.status)}`, item.status)}
                        </span>
                      </div>
                      {item.evidence && <p className="mt-1 text-caption text-muted-foreground line-clamp-2">{item.evidence}</p>}
                      {item.proposal && <p className="mt-1 text-caption text-muted-foreground line-clamp-2">{item.proposal}</p>}
                      <div className="mt-1.5 flex flex-wrap items-center gap-1 text-[10px] text-muted-foreground">
                        <span>{item.problemType}</span>
                        <span>·</span>
                        <span>{item.risk}</span>
                      </div>
                      {canManage && (
                        <div className="mt-2 flex flex-wrap gap-1">
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.id, 'approved')}>
                            <Check className="w-3 h-3 inline mr-0.5" />
                            {t('digitalAvatar.governance.action.approve', '通过')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.id, 'experimenting')}>
                            {t('digitalAvatar.governance.action.experiment', '实验')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.id, 'deployed')}>
                            {t('digitalAvatar.governance.action.deploy', '部署')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.id, 'rolled_back')}>
                            <CircleSlash className="w-3 h-3 inline mr-0.5" />
                            {t('digitalAvatar.governance.action.rollback', '回滚')}
                          </button>
                          <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.id, 'rejected')}>
                            {t('digitalAvatar.governance.action.reject', '拒绝')}
                          </button>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardContent className="py-3">
                <div className="mb-2 rounded-md border bg-muted/20 px-2 py-1.5 text-[10px] text-muted-foreground">
                  {t('digitalAvatar.governance.autoEngineSummary', '自动治理已启用；同类缺口累计 {{count}} 次自动生成新增分身提案。', {
                    count: automationConfig.autoProposalTriggerCount,
                  })}
                </div>
                <div className="mb-2 rounded-md border bg-muted/20 p-2">
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <p className="text-[11px] font-medium text-foreground">
                        {t('digitalAvatar.governance.autoProposalThresholdLabel', '自动提案触发阈值')}
                      </p>
                      <p className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.governance.autoProposalThresholdHint', '同类缺口累计达到该次数后，自动生成新增分身提案（1-10）。')}
                      </p>
                    </div>
                    <div className="flex items-center gap-1.5 shrink-0">
                      <input
                        type="number"
                        min={1}
                        max={10}
                        className="h-7 w-16 rounded border bg-background px-2 text-[11px]"
                        value={autoProposalTriggerCountDraft}
                        onChange={(e) => setAutoProposalTriggerCountDraft(Number(e.target.value || 0))}
                        disabled={!canManage || savingAutomationConfig}
                      />
                      {canManage && (
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-7 px-2 text-[11px]"
                          onClick={saveAutomationConfig}
                          disabled={savingAutomationConfig}
                        >
                          {savingAutomationConfig ? <Loader2 className="w-3 h-3 animate-spin" /> : t('common.save', '保存')}
                        </Button>
                      )}
                    </div>
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-2 text-center">
                  <div className="rounded-md border p-2">
                    <p className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.stats.pendingReq', '待处理请求')}
                    </p>
                    <p className="text-sm font-semibold">{governanceStats.pendingCapability}</p>
                  </div>
                  <div className="rounded-md border p-2">
                    <p className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.stats.pendingProposal', '待审批提案')}
                    </p>
                    <p className="text-sm font-semibold">{governanceStats.pendingProposals}</p>
                  </div>
                  <div className="rounded-md border p-2">
                    <p className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.stats.pendingTicket', '待处理工单')}
                    </p>
                    <p className="text-sm font-semibold">{governanceStats.pendingTickets}</p>
                  </div>
                </div>
                <p className="mt-2 text-caption text-muted-foreground">
                  {managerProcessing
                    ? t('digitalAvatar.governance.managerWorking', '管理 Agent 正在运行，可持续产生优化建议。')
                    : t('digitalAvatar.governance.managerIdle', '管理 Agent 空闲，可发起新一轮能力评估。')}
                </p>
              </CardContent>
            </Card>
          </div>
        </div>
      )}

      <CreateAvatarDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        teamId={teamId}
        managerAgentId={selectedManagerGroupId}
        managerAgentName={getAgentName(agents, selectedManagerGroupId, t('digitalAvatar.labels.unset'))}
        onCreated={(avatar) => {
          addToast('success', t('common.created'));
          setCreateOpen(false);
          setSelectedAvatarId(avatar.id);
          loadAvatars(false);
        }}
      />
      <CreateManagerAgentDialog
        open={createManagerOpen}
        onOpenChange={setCreateManagerOpen}
        teamId={teamId}
        onCreated={(agent) => {
          addToast('success', t('common.created'));
          setCreateManagerOpen(false);
          setAgents((prev) => [agent, ...prev.filter((item) => item.id !== agent.id)]);
          setBootstrapManagerAgentId(agent.id);
          setSelectedAvatarId(null);
        }}
      />
    </div>
  );
}
