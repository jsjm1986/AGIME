import { useState, useEffect, useCallback, useRef, useMemo, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useToast } from '../../contexts/ToastContext';
import { Button } from '../ui/button';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { SearchInput } from '../ui/search-input';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/select';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '../ui/table';
import MarkdownContent from '../MarkdownContent';
import { MissionCard, MissionCardMedium } from '../mission/MissionCard';
import { StatusBadge, MISSION_STATUS_MAP } from '../ui/status-badge';
import { CreateMissionDialog } from '../mission/CreateMissionDialog';
import { MissionStepList } from '../mission/MissionStepList';
import { MissionStepDetail } from '../mission/MissionStepDetail';
import { ArtifactList } from '../mission/ArtifactList';
import { MissionEventList } from '../mission/MissionEventList';
import { StepApprovalPanel } from '../mission/StepApprovalPanel';
import { GoalTreeView } from '../mission/GoalTreeView';
import {
  missionApi,
  MissionDetail,
  MissionListItem,
  MissionMonitorSnapshot,
  MissionStatus,
  GoalStatus,
  GoalNode,
  MissionStep,
} from '../../api/mission';
import { ApiError } from '../../api/client';
import { localizeMissionError } from '../../utils/missionError';
import { formatDate } from '../../utils/format';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { ManagementRail } from '../mobile/ManagementRail';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

type DetailTab = 'work' | 'artifacts' | 'evidence' | 'logs';

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function formatDuration(ms: number): string {
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  if (m < 60) return `${m}m ${sec % 60}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m`;
}

function humanizeToken(value?: string | null): string {
  if (!value) return '';
  return value
    .replace(/_/g, ' ')
    .replace(/\b\w/g, ch => ch.toUpperCase());
}

const ACTIVE_STATUSES: MissionStatus[] = ['planning', 'planned', 'running'];
const HISTORY_STATUSES: MissionStatus[] = ['completed', 'paused', 'draft', 'failed', 'cancelled'];

function isAdaptiveMissionListItem(mission: MissionListItem): boolean {
  return mission.goal_count > 0;
}

function isAdaptiveMissionDetail(mission: MissionDetail): boolean {
  return Boolean(
    (mission.goal_tree?.length ?? 0) > 0 ||
    mission.current_goal_id ||
    mission.total_pivots > 0 ||
    mission.total_abandoned > 0,
  );
}

interface MissionsPanelProps {
  teamId: string;
}

export function MissionsPanel({ teamId }: MissionsPanelProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const navigate = useNavigate();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();
  const isConversationTaskMode = isConversationMode && isMobileWorkspace;

  // Board state
  const [missions, setMissions] = useState<MissionListItem[]>([]);
  const [boardLoading, setBoardLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [agentFilter, setAgentFilter] = useState('__all__');
  const [historyStatusFilter, setHistoryStatusFilter] = useState<MissionStatus | '__all__'>('__all__');
  const [viewMode, setViewMode] = useState<'board' | 'list'>('board');
  const [mobileMissionBoardOpen, setMobileMissionBoardOpen] = useState(false);
  const [mobileFilterSheetOpen, setMobileFilterSheetOpen] = useState(false);

  // Detail state
  const [selectedMissionId, setSelectedMissionId] = useState<string | null>(null);
  const [mission, setMission] = useState<MissionDetail | null>(null);
  const [monitorSnapshot, setMonitorSnapshot] = useState<MissionMonitorSnapshot | null>(null);

  const [messages, setMessages] = useState<StreamMessage[]>([]);
  const [activeTab, setActiveTab] = useState<DetailTab>('artifacts');
  const [startPending, setStartPending] = useState(false);

  // Dialog state
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [pivotDialogOpen, setPivotDialogOpen] = useState(false);
  const [pivotGoalId, setPivotGoalId] = useState<string | null>(null);
  const [pivotApproach, setPivotApproach] = useState('');
  const [resumeDialogOpen, setResumeDialogOpen] = useState(false);
  const [resumeFeedback, setResumeFeedback] = useState('');
  const eventSourceRef = useRef<EventSource | null>(null);
  const lastEventIdRef = useRef<number | null>(null);
  const seenEventIdsRef = useRef<Set<string>>(new Set());
  const activeRunIdRef = useRef<string | null>(null);
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef<number | null>(null);

  /** Reset SSE reconnect state. */
  const resetReconnectState = useCallback(() => {
    lastEventIdRef.current = null;
    seenEventIdsRef.current.clear();
    reconnectAttemptsRef.current = 0;
    if (reconnectTimerRef.current) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  // Load board
  const loadMissions = useCallback(async () => {
    try {
      const items = await missionApi.listMissions(teamId, undefined, undefined, 1, 100);
      setMissions(items || []);
    } catch (e) {
      console.error('Failed to load missions:', e);
    } finally {
      setBoardLoading(false);
    }
  }, [teamId]);

  useEffect(() => {
    loadMissions();
  }, [loadMissions]);

  useEffect(() => {
    const refresh = () => {
      void loadMissions();
    };

    const intervalId = window.setInterval(refresh, 15000);
    const handleFocus = () => refresh();
    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        refresh();
      }
    };

    window.addEventListener('focus', handleFocus);
    document.addEventListener('visibilitychange', handleVisibilityChange);
    return () => {
      window.clearInterval(intervalId);
      window.removeEventListener('focus', handleFocus);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [loadMissions]);

  // Load mission detail
  const loadMission = useCallback(async () => {
    if (!selectedMissionId) return;
    try {
      const [data, snapshot] = await Promise.all([
        missionApi.getMission(selectedMissionId),
        missionApi.getMonitorSnapshot(selectedMissionId).catch(() => null),
      ]);
      setMission(data);
      setMonitorSnapshot(snapshot);
    } catch (e) {
      console.error('Failed to load mission:', e);
    }
  }, [selectedMissionId]);

  useEffect(() => {
    if (selectedMissionId) {
      setMessages([]);
      setActiveTab('artifacts');
      setMonitorSnapshot(null);
      resetReconnectState();
      loadMission();
    }
  }, [selectedMissionId, loadMission, resetReconnectState]);

  // Poll mission detail for active missions
  useEffect(() => {
    if (!selectedMissionId || !mission) return;
    const isLive = ['planning', 'planned', 'running'].includes(mission.status);
    if (!isLive) return;
    const timer = setInterval(loadMission, 3000);
    return () => clearInterval(timer);
  }, [selectedMissionId, mission?.status, loadMission]);

  // SSE streaming for detail view
  useEffect(() => {
    if (!selectedMissionId || !mission) return;
    const isLive = ['planning', 'running'].includes(mission.status);
    if (!isLive) return;

    let cancelled = false;

    const shouldHandleEvent = (e: Event) => {
      const raw = (e as MessageEvent).lastEventId;
      const parsed = Number(raw || 0);
      if (Number.isFinite(parsed) && parsed > 0) {
        const runKey = mission.current_run_id || activeRunIdRef.current || 'legacy';
        const dedupeKey = `${runKey}:${parsed}`;
        const seen = seenEventIdsRef.current;
        if (seen.has(dedupeKey)) return false;
        seen.add(dedupeKey);
        if (seen.size > 10_000) {
          seen.clear();
          seen.add(dedupeKey);
        }
        lastEventIdRef.current = parsed;
      }
      return true;
    };

    const connectStream = (isReconnect = false) => {
      if (cancelled) return;

      eventSourceRef.current?.close();
      eventSourceRef.current = null;
      if (!isReconnect) {
        reconnectAttemptsRef.current = 0;
      }

      const es = missionApi.streamMission(selectedMissionId, lastEventIdRef.current);
      eventSourceRef.current = es;

      const appendMessage = (type: string, content: string) => {
        const raw = content || '';
        const normalized = raw.trim();
        if (!normalized) return;
        const signature = normalized.replace(/\s+/g, ' ');
        setMessages(prev => {
          const now = Date.now();
          const last = prev[prev.length - 1];
          if (
            last &&
            last.type === type &&
            last.content.trim().replace(/\s+/g, ' ') === signature
          ) {
            return prev;
          }

          // Suppress replayed chunks on reconnect/history overlap.
          // Scan a larger recent window so duplicated sequences are filtered
          // even when they are not strictly adjacent.
          for (let i = prev.length - 1, scanned = 0; i >= 0 && scanned < 80; i--, scanned++) {
            const item = prev[i];
            if (now - item.timestamp > 30_000) break;
            if (item.type !== type) continue;
            if (item.content.trim().replace(/\s+/g, ' ') === signature) {
              return prev;
            }
          }

          return [...prev, { type, content: raw, timestamp: now }];
        });
      };

      const handleEvent = (type: string) => (e: MessageEvent) => {
        if (!shouldHandleEvent(e)) return;
        try {
          const data = JSON.parse(e.data);
          if (type === 'status') {
            loadMission();
            return;
          }
          if (type === 'done') {
            reconnectAttemptsRef.current = 0;
            if (reconnectTimerRef.current) {
              window.clearTimeout(reconnectTimerRef.current);
              reconnectTimerRef.current = null;
            }
            loadMission();
            es.close();
            eventSourceRef.current = null;
            return;
          }
          appendMessage(
            type,
            data.text || data.content || data.tool_name || JSON.stringify(data),
          );
        } catch {
          // ignore parse errors
        }
      };

      const handleGoalStart = (e: MessageEvent) => {
        if (!shouldHandleEvent(e)) return;
        try {
          const data = JSON.parse(e.data);
          setMission(prev => {
            if (!prev?.goal_tree) return prev;
            return {
              ...prev,
              current_goal_id: data.goal_id,
              goal_tree: prev.goal_tree.map(g =>
                g.goal_id === data.goal_id ? { ...g, status: 'running' as GoalStatus } : g
              ),
            };
          });
          appendMessage('goal_start', `▶ ${data.goal_id}: ${data.title}`);
        } catch {
          // ignore parse errors
        }
      };

      const handleGoalComplete = (e: MessageEvent) => {
        if (!shouldHandleEvent(e)) return;
        try {
          const data = JSON.parse(e.data);
          setMission(prev => {
            if (!prev?.goal_tree) return prev;
            return {
              ...prev,
              goal_tree: prev.goal_tree.map(g =>
                g.goal_id === data.goal_id ? { ...g, status: 'completed' as GoalStatus } : g
              ),
            };
          });
          appendMessage('goal_complete', `✓ ${data.goal_id} (${data.signal})`);
        } catch {
          // ignore parse errors
        }
      };

      const handlePivot = (e: MessageEvent) => {
        if (!shouldHandleEvent(e)) return;
        try {
          const data = JSON.parse(e.data);
          setMission(prev => {
            if (!prev?.goal_tree) return prev;
            return {
              ...prev,
              total_pivots: prev.total_pivots + 1,
              goal_tree: prev.goal_tree.map(g =>
                g.goal_id === data.goal_id
                  ? { ...g, status: 'pivoting' as GoalStatus, pivot_reason: data.to_approach }
                  : g
              ),
            };
          });
          appendMessage(
            'pivot',
            `↻ ${data.goal_id}: ${data.from_approach} → ${data.to_approach}`,
          );
        } catch {
          // ignore parse errors
        }
      };

      const handleGoalAbandoned = (e: MessageEvent) => {
        if (!shouldHandleEvent(e)) return;
        try {
          const data = JSON.parse(e.data);
          setMission(prev => {
            if (!prev?.goal_tree) return prev;
            return {
              ...prev,
              total_abandoned: prev.total_abandoned + 1,
              goal_tree: prev.goal_tree.map(g =>
                g.goal_id === data.goal_id
                  ? { ...g, status: 'abandoned' as GoalStatus, pivot_reason: data.reason }
                  : g
              ),
            };
          });
          appendMessage('goal_abandoned', `⊘ ${data.goal_id}: ${data.reason}`);
        } catch {
          // ignore parse errors
        }
      };

      es.addEventListener('text', handleEvent('text'));
      es.addEventListener('thinking', handleEvent('thinking'));
      es.addEventListener('toolcall', handleEvent('toolcall'));
      es.addEventListener('toolresult', handleEvent('toolresult'));
      es.addEventListener('status', handleEvent('status'));
      es.addEventListener('done', handleEvent('done'));
      es.addEventListener('goal_start', handleGoalStart);
      es.addEventListener('goal_complete', handleGoalComplete);
      es.addEventListener('pivot', handlePivot);
      es.addEventListener('goal_abandoned', handleGoalAbandoned);
      es.onerror = () => {
        es.close();
        eventSourceRef.current = null;
        if (cancelled) return;

        const nextAttempt = reconnectAttemptsRef.current + 1;
        reconnectAttemptsRef.current = nextAttempt;
        const delay = Math.min(1000 * nextAttempt, 5000);
        if (reconnectTimerRef.current) {
          window.clearTimeout(reconnectTimerRef.current);
        }
        reconnectTimerRef.current = window.setTimeout(async () => {
          if (cancelled) return;
          try {
            const detail = await missionApi.getMission(selectedMissionId);
            if (cancelled) return;
            setMission(detail);
            if (['planning', 'running'].includes(detail.status)) {
              connectStream(true);
            } else {
              reconnectAttemptsRef.current = 0;
            }
          } catch (err) {
            if (err instanceof ApiError && (err.status === 403 || err.status === 404)) {
              reconnectAttemptsRef.current = 0;
              return;
            }
            if (!cancelled) {
              connectStream(true);
            }
          }
        }, delay);
      };
    };

    connectStream(false);

    return () => {
      cancelled = true;
      eventSourceRef.current?.close();
      eventSourceRef.current = null;
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
    };
  }, [selectedMissionId, mission?.status, loadMission]);

  // Reset event de-duplication when mission execution run changes.
  useEffect(() => {
    const nextRunId = mission?.current_run_id ?? null;
    if (nextRunId === activeRunIdRef.current) return;
    activeRunIdRef.current = nextRunId;
    setMessages([]);
    resetReconnectState();
  }, [mission?.current_run_id, resetReconnectState]);

  // Action handlers
  const handleStart = async () => {
    if (!selectedMissionId || startPending) return;
    if (mission?.status === 'failed') {
      setResumeFeedback('');
      setResumeDialogOpen(true);
      return;
    }
    setStartPending(true);
    try {
      if (mission?.status === 'paused') {
        const res = await missionApi.resumeMission(selectedMissionId);
        if (res.status === 'pause_in_progress') {
          addToast('info', t('mission.pauseInProgress', 'Pause is still being applied, try resume again shortly'));
        }
      } else {
        await missionApi.startMission(selectedMissionId);
      }
      setMessages([]);
      resetReconnectState();
      await loadMission();
      await loadMissions();
    } catch {
      addToast('error', t('mission.startFailed', 'Failed to start mission'));
    } finally {
      setStartPending(false);
    }
  };

  const confirmResumeFromFailed = async () => {
    if (!selectedMissionId || startPending) return;
    setStartPending(true);
    try {
      await missionApi.resumeMission(
        selectedMissionId,
        resumeFeedback.trim() || undefined,
      );
      setResumeDialogOpen(false);
      setMessages([]);
      resetReconnectState();
      await loadMission();
      await loadMissions();
    } catch {
      addToast('error', t('mission.startFailed', 'Failed to start mission'));
    } finally {
      setStartPending(false);
    }
  };

  const handlePauseMission = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.pauseMission(selectedMissionId);
      loadMission();
    } catch { addToast('error', t('mission.pauseFailed', 'Failed to pause mission')); }
  };

  const handleCancelMission = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.cancelMission(selectedMissionId);
      loadMission();
    } catch { addToast('error', t('mission.cancelFailed', 'Failed to cancel mission')); }
  };

  const handleDeleteMission = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.deleteMission(selectedMissionId);
      setSelectedMissionId(null);
      setMission(null);
      setDeleteConfirmOpen(false);
      loadMissions();
    } catch { addToast('error', t('mission.deleteFailed', 'Failed to delete mission')); }
  };

  const handleApproveStep = async (stepIndex: number, feedback?: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.approveStep(selectedMissionId, stepIndex, feedback);
      setMessages([]);
      resetReconnectState();
      loadMission();
    } catch { addToast('error', t('mission.approveFailed', 'Failed to approve step')); }
  };

  const handleRejectStep = async (stepIndex: number, feedback?: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.rejectStep(selectedMissionId, stepIndex, feedback);
      loadMission();
    } catch { addToast('error', t('mission.rejectFailed', 'Failed to reject step')); }
  };

  const handleSkipStep = async (stepIndex: number) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.skipStep(selectedMissionId, stepIndex);
      loadMission();
    } catch { addToast('error', t('mission.skipFailed', 'Failed to skip step')); }
  };

  const handleApproveGoal = async (goalId: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.approveGoal(selectedMissionId, goalId);
      setMessages([]);
      resetReconnectState();
      loadMission();
    } catch { addToast('error', t('mission.approveGoalFailed', 'Failed to approve goal')); }
  };

  const handleRejectGoal = async (goalId: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.rejectGoal(selectedMissionId, goalId);
      loadMission();
    } catch { addToast('error', t('mission.rejectGoalFailed', 'Failed to reject goal')); }
  };

  const handlePivotGoal = async (goalId: string) => {
    setPivotGoalId(goalId);
    setPivotApproach('');
    setPivotDialogOpen(true);
  };

  const confirmPivotGoal = async () => {
    if (!selectedMissionId || !pivotGoalId || !pivotApproach) return;
    try {
      await missionApi.pivotGoal(selectedMissionId, pivotGoalId, pivotApproach);
      setPivotDialogOpen(false);
      loadMission();
    } catch { addToast('error', t('mission.pivotFailed', 'Failed to pivot goal')); }
  };

  const handleAbandonGoal = async (goalId: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.abandonGoal(selectedMissionId, goalId, t('mission.abandonedByUser'));
      loadMission();
    } catch { addToast('error', t('mission.abandonFailed', 'Failed to abandon goal')); }
  };

  const handleCreate = async (data: Parameters<typeof missionApi.createMission>[0]) => {
    try {
      const res = await missionApi.createMission(data);
      setShowCreate(false);
      if (res.route === 'direct') {
        addToast(
          'info',
          t(
            'mission.routedToDirect',
            'This request was routed to direct chat for faster execution.',
          ),
        );
        window.location.assign(`/admin/teams/${teamId}?section=chat`);
        return;
      }
      loadMissions();
    } catch { addToast('error', t('mission.createFailed', 'Failed to create mission')); }
  };

  const handleBack = () => {
    eventSourceRef.current?.close();
    eventSourceRef.current = null;
    resetReconnectState();
    setSelectedMissionId(null);
    setMission(null);
    setMessages([]);
    loadMissions();
  };

  // Quick actions from board cards
  const handleQuickStart = async (missionId: string) => {
    try {
      const m = missions.find(x => x.mission_id === missionId);
      if (m?.status === 'paused' || m?.status === 'failed') {
        const res = await missionApi.resumeMission(missionId);
        if (res.status === 'pause_in_progress') {
          addToast('info', t('mission.pauseInProgress'));
        }
      } else {
        await missionApi.startMission(missionId);
      }
      loadMissions();
    } catch { addToast('error', t('mission.startFailed')); }
  };

  const handleQuickPause = async (missionId: string) => {
    try {
      await missionApi.pauseMission(missionId);
      loadMissions();
    } catch { addToast('error', t('mission.pauseFailed')); }
  };

  // Filtered missions
  const filteredMissions = useMemo(() => {
    let list = missions;
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      list = list.filter(m => m.goal.toLowerCase().includes(q) || m.agent_name.toLowerCase().includes(q));
    }
    if (agentFilter !== '__all__') {
      list = list.filter(m => m.agent_name === agentFilter);
    }
    return list;
  }, [missions, searchQuery, agentFilter]);

  // Aggregate stats (per-status counts live in column headers)
  const stats = useMemo(() => {
    const completed = missions.filter(m => m.status === 'completed').length;
    const failed = missions.filter(m => m.status === 'failed').length;
    const finished = completed + failed;
    const rate = finished > 0 ? Math.round((completed / finished) * 100) : 0;
    const totalTokens = missions.reduce((sum, m) => sum + m.total_tokens_used, 0);
    return { rate, totalTokens };
  }, [missions]);

  // Unique agent names for filter
  const agentNames = useMemo(() => [...new Set(missions.map(m => m.agent_name))], [missions]);

  const activeMissions = useMemo(
    () => filteredMissions.filter(m => ACTIVE_STATUSES.includes(m.status)),
    [filteredMissions],
  );

  const historyMissions = useMemo(() => {
    const list = filteredMissions.filter(m => !ACTIVE_STATUSES.includes(m.status));
    if (historyStatusFilter === '__all__') {
      return list.sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
    }
    return list
      .filter(m => m.status === historyStatusFilter)
      .sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime());
  }, [filteredMissions, historyStatusFilter]);

  const missionFilterPanel = (
    <div className="space-y-4">
      <SearchInput
        placeholder={t('mission.searchMissions')}
        value={searchQuery}
        onChange={(e) => setSearchQuery(e.target.value)}
        onClear={() => setSearchQuery('')}
        className="h-10 text-sm"
      />
      {agentNames.length > 1 && (
        <div className="space-y-2">
          <div className="text-[11px] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
            {t('mission.filterAgent')}
          </div>
          <Select value={agentFilter} onValueChange={setAgentFilter}>
            <SelectTrigger className="h-10">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all__">{t('mission.filterAgent')}</SelectItem>
              {agentNames.map(name => (
                <SelectItem key={name} value={name}>{name}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
      <div className="space-y-2">
        <div className="text-[11px] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
          {t('mission.history')}
        </div>
        <Select value={historyStatusFilter} onValueChange={(v) => setHistoryStatusFilter(v as MissionStatus | '__all__')}>
          <SelectTrigger className="h-10">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{t('mission.allStatuses')}</SelectItem>
            {HISTORY_STATUSES.map(s => (
              <SelectItem key={s} value={s}>{t(`mission.${s}`)}</SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <div className="text-[11px] font-semibold tracking-[0.12em] text-muted-foreground uppercase">
          {t('mission.viewMode', '视图方式')}
        </div>
        <div className="grid grid-cols-2 gap-2">
          {(['board', 'list'] as const).map((mode) => (
            <button
              key={mode}
              type="button"
              onClick={() => setViewMode(mode)}
              className={`rounded-[18px] border px-3 py-2 text-sm font-medium transition-colors ${
                viewMode === mode
                  ? 'border-primary/40 bg-primary/10 text-primary'
                  : 'border-border/70 bg-background/80 text-muted-foreground'
              }`}
            >
              {t(`mission.view${mode === 'board' ? 'Board' : 'List'}`)}
            </button>
          ))}
        </div>
      </div>
    </div>
  );

  const mobileMissionBoardContent = (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-2">
        <Button size="sm" variant="outline" className="h-9 rounded-[14px] text-[11px]" onClick={() => setShowCreate(true)}>
          + {t('mission.create')}
        </Button>
        <Button size="sm" variant="outline" className="h-9 rounded-[14px] text-[11px]" onClick={() => {
          setMobileMissionBoardOpen(false);
          setMobileFilterSheetOpen(true);
        }}>
          {t('mission.quickFilters', '筛选与视图')}
        </Button>
      </div>

      {activeMissions[0] ? (
        <div className="rounded-[18px] border border-border/65 bg-background/88 px-3.5 py-3">
          <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            {t('mission.currentMission', '当前活跃任务')}
          </div>
          <div className="mt-1 text-[12px] font-semibold text-foreground">
            {activeMissions[0].goal}
          </div>
          <div className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
            {activeMissions[0].agent_name} · {t(`mission.${activeMissions[0].status}`)}
          </div>
          <div className="mt-3 grid grid-cols-2 gap-2">
            <Button
              size="sm"
              variant="outline"
              className="h-8 rounded-[12px] text-[11px]"
              onClick={() => {
                setSelectedMissionId(activeMissions[0].mission_id);
                setMobileMissionBoardOpen(false);
              }}
            >
              {t('mission.openCurrentMission', '打开任务详情')}
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="h-8 rounded-[12px] text-[11px]"
              onClick={() => {
                setSelectedMissionId(activeMissions[0].mission_id);
                setActiveTab('artifacts');
                setMobileMissionBoardOpen(false);
              }}
            >
              {t('mission.openArtifactsWorkspace', '查看任务产物')}
            </Button>
          </div>
        </div>
      ) : null}

      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            {t('mission.statsActive')}
          </div>
          <span className="rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.7] px-2 py-1 text-[10px] font-semibold text-foreground">
            {activeMissions.length}
          </span>
        </div>
        {activeMissions.length > 0 ? (
          <div className="space-y-2">
            {activeMissions.slice(0, 4).map((item) => (
              <MissionCardMedium
                key={item.mission_id}
                mission={item}
                onClick={(id) => {
                  setSelectedMissionId(id);
                  setMobileMissionBoardOpen(false);
                }}
              />
            ))}
          </div>
        ) : (
          <div className="rounded-[16px] border border-dashed border-border/70 px-3.5 py-4 text-[11px] text-muted-foreground">
            {t('mission.noActiveMissions')}
          </div>
        )}
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3">
          <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            {t('mission.history')}
          </div>
          <span className="rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.7] px-2 py-1 text-[10px] font-semibold text-foreground">
            {historyMissions.length}
          </span>
        </div>
        {historyMissions.length > 0 ? (
          <div className="space-y-2">
            {historyMissions.slice(0, 6).map((item) => (
              <MissionCardMedium
                key={item.mission_id}
                mission={item}
                onClick={(id) => {
                  setSelectedMissionId(id);
                  setMobileMissionBoardOpen(false);
                }}
              />
            ))}
          </div>
        ) : (
          <div className="rounded-[16px] border border-dashed border-border/70 px-3.5 py-4 text-[11px] text-muted-foreground">
            {t('mission.noMissions')}
          </div>
        )}
      </div>
    </div>
  );

  const detailOverlays = (
    <>
      <ConfirmDialog
        open={deleteConfirmOpen}
        onOpenChange={setDeleteConfirmOpen}
        title={t('mission.deleteConfirm')}
        variant="destructive"
        onConfirm={handleDeleteMission}
      />
      <ConfirmDialog
        open={pivotDialogOpen}
        onOpenChange={setPivotDialogOpen}
        title={t('mission.pivot')}
        confirmText={t('common.confirm')}
        onConfirm={confirmPivotGoal}
      >
        <input
          className="w-full rounded-md border border-[hsl(var(--input))] bg-[hsl(var(--background))] px-3 py-2 text-sm"
          placeholder={t('mission.pivot')}
          aria-label={t('mission.pivot')}
          value={pivotApproach}
          onChange={(e) => setPivotApproach(e.target.value)}
          autoFocus
        />
      </ConfirmDialog>
      <ConfirmDialog
        open={resumeDialogOpen}
        onOpenChange={setResumeDialogOpen}
        title={t('mission.resumeFromFailed', 'Continue Failed Mission')}
        confirmText={t('mission.resume', 'Resume')}
        onConfirm={confirmResumeFromFailed}
      >
        <textarea
          className="w-full min-h-[96px] rounded-md border border-[hsl(var(--input))] bg-[hsl(var(--background))] px-3 py-2 text-sm"
          placeholder={t(
            'mission.resumeFeedbackPlaceholder',
            'Optional: add guidance for this retry run (for example: prioritize concise report, avoid web tools, focus on data validation first)',
          )}
          aria-label={t('mission.resumeFromFailed', 'Continue Failed Mission')}
          value={resumeFeedback}
          onChange={(e) => setResumeFeedback(e.target.value)}
          autoFocus
        />
      </ConfirmDialog>
    </>
  );

  // ─── Detail View ───
  if (selectedMissionId && mission) {
    if (isConversationTaskMode) {
      const missionAgentName =
        missions.find((item) => item.mission_id === selectedMissionId)?.agent_name ?? mission.agent_id;

      return (
        <>
          <MobileWorkspaceShell
            summary={(
              <ContextSummaryBar
                eyebrow={t('teamNav.missions')}
                title={mission.goal}
                description={t(
                  'mission.mobileDetailSummary',
                  '任务详情成为主舞台，任务流、产物和日志作为当前 Agent 工作的执行上下文。',
                )}
                metrics={[
                  { label: t('mission.listHeaderStatus'), value: t(`mission.${mission.status}`) },
                  { label: t('mission.listHeaderAgent'), value: missionAgentName },
                  { label: t('mission.steps', 'Steps'), value: mission.steps.length },
                  { label: t('mission.totalTokens', 'Tokens'), value: mission.total_tokens_used > 0 ? formatTokens(mission.total_tokens_used) : '—' },
                ]}
                actions={(
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-full px-3"
                    onClick={handleBack}
                  >
                    {t('common.back', '返回')}
                  </Button>
                )}
              />
            )}
            actions={(
              <div className="grid grid-cols-2 gap-2">
                <Button variant="outline" className="h-10 justify-start rounded-[16px] px-3" onClick={handleBack}>
                  {t('mission.backToBoard', '回到任务列表')}
                </Button>
                <Button
                  variant="outline"
                  className="h-10 justify-start rounded-[16px] px-3"
                  onClick={() => navigate(`/teams/${teamId}?section=chat`)}
                >
                  {t('mission.quickChatAssist', '回到对话继续处理')}
                </Button>
                <Button
                  variant="outline"
                  className="h-10 justify-start rounded-[16px] px-3"
                  onClick={() => setActiveTab('artifacts')}
                >
                  {t('mission.viewArtifacts', '查看产物')}
                </Button>
                <Button
                  variant="outline"
                  className="h-10 justify-start rounded-[16px] px-3"
                  onClick={() => setActiveTab('logs')}
                >
                  {t('mission.logs', '日志')}
                </Button>
              </div>
            )}
            stage={(
              <MissionDetailView
                teamId={teamId}
                mission={mission}
                monitorSnapshot={monitorSnapshot}
                missionId={selectedMissionId}
                messages={messages}
                activeTab={activeTab}
                setActiveTab={setActiveTab}
                startPending={startPending}
                onBack={handleBack}
                onStart={handleStart}
                onPause={handlePauseMission}
                onCancel={handleCancelMission}
                onDelete={() => setDeleteConfirmOpen(true)}
                onApproveStep={handleApproveStep}
                onRejectStep={handleRejectStep}
                onSkipStep={handleSkipStep}
                onApproveGoal={handleApproveGoal}
                onRejectGoal={handleRejectGoal}
                onPivotGoal={handlePivotGoal}
                onAbandonGoal={handleAbandonGoal}
              />
            )}
            rail={(
              <ManagementRail
                title={t('mission.executionContext', '执行上下文')}
                description={t(
                  'mission.executionContextHint',
                  '当前任务继续占据主舞台，其余协同动作退到这里和抽屉层中。',
                )}
              >
                <div className="grid grid-cols-2 gap-3">
                  <div className="rounded-[18px] border border-border/60 bg-background/80 px-4 py-3">
                    <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/72">
                      {t('mission.currentTab', '当前标签')}
                    </div>
                    <div className="mt-1 text-[13px] font-semibold text-foreground">
                      {t(`mission.tab.${activeTab}`, activeTab)}
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-border/60 bg-background/80 px-4 py-3">
                    <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/72">
                      {t('mission.completed', 'Completed')}
                    </div>
                    <div className="mt-1 text-[13px] font-semibold text-foreground">
                      {mission.steps.filter((step) => step.status === 'completed').length}/{mission.steps.length}
                    </div>
                  </div>
                </div>
              </ManagementRail>
            )}
            panel={detailOverlays}
          />
        </>
      );
    }

    return (
      <>
        <MissionDetailView
          teamId={teamId}
          mission={mission}
          monitorSnapshot={monitorSnapshot}
          missionId={selectedMissionId}
          messages={messages}
          activeTab={activeTab}
          setActiveTab={setActiveTab}
          startPending={startPending}
          onBack={handleBack}
          onStart={handleStart}
          onPause={handlePauseMission}
          onCancel={handleCancelMission}
          onDelete={() => setDeleteConfirmOpen(true)}
          onApproveStep={handleApproveStep}
          onRejectStep={handleRejectStep}
          onSkipStep={handleSkipStep}
          onApproveGoal={handleApproveGoal}
          onRejectGoal={handleRejectGoal}
          onPivotGoal={handlePivotGoal}
          onAbandonGoal={handleAbandonGoal}
        />
        {detailOverlays}
      </>
    );
  }

  // ─── Board View ───
  const renderHistoryStatusFilter = (triggerClassName: string) => (
    viewMode === 'board' ? (
      <Select value={historyStatusFilter} onValueChange={(v) => setHistoryStatusFilter(v as MissionStatus | '__all__')}>
        <SelectTrigger className={triggerClassName}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="__all__">{t('mission.allStatuses')}</SelectItem>
          {HISTORY_STATUSES.map(s => (
            <SelectItem key={s} value={s}>{t(`mission.${s}`)}</SelectItem>
          ))}
        </SelectContent>
      </Select>
    ) : null
  );

  const boardBody = boardLoading ? (
    <div className="flex-1 flex items-center justify-center">
      <p className="text-muted-foreground">{t('common.loading')}</p>
    </div>
  ) : missions.length === 0 ? (
    <div className="flex-1 flex items-center justify-center">
      <div className="text-center">
        <p className="text-muted-foreground mb-4">{t('mission.noMissions')}</p>
        <Button size="sm" onClick={() => setShowCreate(true)}>
          + {t('mission.create')}
        </Button>
      </div>
    </div>
  ) : viewMode === 'list' ? (
    <div className="flex-1 overflow-auto px-3 sm:px-5 py-3">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="text-caption uppercase tracking-wider font-medium w-20 sm:w-28">{t('mission.listHeaderStatus')}</TableHead>
            <TableHead className="text-caption uppercase tracking-wider font-medium">{t('mission.listHeaderGoal')}</TableHead>
            <TableHead className="text-caption uppercase tracking-wider font-medium w-32 hidden md:table-cell">{t('mission.listHeaderAgent')}</TableHead>
            <TableHead className="text-caption uppercase tracking-wider font-medium w-36 hidden sm:table-cell">{t('mission.listHeaderProgress')}</TableHead>
            <TableHead className="text-caption uppercase tracking-wider font-medium w-20 hidden md:table-cell">{t('mission.listHeaderTokens')}</TableHead>
            <TableHead className="text-caption uppercase tracking-wider font-medium w-28 hidden lg:table-cell">{t('mission.listHeaderCreated')}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {filteredMissions.map(m => {
            const isAdaptive = isAdaptiveMissionListItem(m);
            const total = isAdaptive ? m.goal_count : m.step_count;
            const done = isAdaptive ? m.completed_goals : m.completed_steps;
            const pct = total > 0 ? Math.round((done / total) * 100) : 0;
            return (
              <TableRow key={m.mission_id} className="cursor-pointer group" onClick={() => setSelectedMissionId(m.mission_id)}>
                <TableCell className="py-2.5 sm:py-3">
                  <StatusBadge status={MISSION_STATUS_MAP[m.status] || 'neutral'}>
                    {t(`mission.${m.status}`)}
                  </StatusBadge>
                </TableCell>
                <TableCell className="py-2.5 sm:py-3 max-w-[140px] sm:max-w-xs">
                  <span className="text-[13px] font-medium truncate block group-hover:text-foreground">{m.goal}</span>
                </TableCell>
                <TableCell className="py-3 text-[12px] text-muted-foreground/70 hidden md:table-cell">{m.agent_name}</TableCell>
                <TableCell className="py-3 hidden sm:table-cell">
                  {total > 0 && (
                    <div className="flex items-center gap-2">
                      <div className="w-16 h-1 bg-muted/80 rounded-full overflow-hidden">
                        <div className="h-full bg-foreground/20 rounded-full" style={{ width: `${pct}%` }} />
                      </div>
                      <span className="text-caption tabular-nums text-muted-foreground/75">{pct}%</span>
                    </div>
                  )}
                </TableCell>
                <TableCell className="hidden py-3 text-caption tabular-nums text-muted-foreground/70 md:table-cell">{m.total_tokens_used > 0 ? formatTokens(m.total_tokens_used) : '—'}</TableCell>
                <TableCell className="hidden py-3 text-caption tabular-nums text-muted-foreground/70 lg:table-cell">{formatDate(m.created_at)}</TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </div>
  ) : (
    <div className="flex-1 overflow-auto p-3 sm:p-4 space-y-4">
      {activeMissions.length > 0 ? (
        <div>
          <div className="flex items-center gap-2 mb-2">
            <h3 className="text-caption font-semibold uppercase tracking-wider text-muted-foreground/70">{t('mission.statsActive')}</h3>
            <span className="rounded bg-muted/60 px-1.5 py-0.5 text-caption tabular-nums text-muted-foreground/70">
              {activeMissions.length}
            </span>
          </div>
          <div className="flex gap-3 overflow-x-auto pb-2">
            {activeMissions.map(m => (
              <div key={m.mission_id} className="w-72 shrink-0">
                <MissionCard
                  mission={m}
                  onClick={(id) => setSelectedMissionId(id)}
                  onQuickStart={handleQuickStart}
                  onQuickPause={handleQuickPause}
                />
              </div>
            ))}
          </div>
        </div>
      ) : (
        <p className="py-1 text-[12px] text-muted-foreground/65">{t('mission.noActiveMissions')}</p>
      )}

      <div className="border-t border-border/40" />

      <div>
        <div className="flex items-center gap-2 mb-3">
          <h3 className="text-caption font-semibold uppercase tracking-wider text-muted-foreground/70">{t('mission.history')}</h3>
          <span className="rounded bg-muted/60 px-1.5 py-0.5 text-caption tabular-nums text-muted-foreground/70">
            {historyMissions.length}
          </span>
        </div>
        {historyMissions.length > 0 ? (
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
            {historyMissions.map(m => (
              <MissionCardMedium
                key={m.mission_id}
                mission={m}
                onClick={(id) => setSelectedMissionId(id)}
              />
            ))}
          </div>
        ) : (
          <p className="text-[12px] text-muted-foreground/65">{t('mission.noMissions')}</p>
        )}
      </div>
    </div>
  );

  if (isConversationTaskMode) {
    const activeMission = activeMissions[0] ?? null;

    return (
      <>
        <MobileWorkspaceShell
          summary={(
            <ContextSummaryBar
              eyebrow={t('teamNav.missions')}
              title={t('mission.title')}
              description={t(
                'mission.mobileSummaryDescription',
                '任务继续作为执行单元存在，但首屏优先保留当前任务线索和进入入口。',
              )}
              metrics={[
                { label: t('mission.statsActive'), value: activeMissions.length },
                { label: t('mission.history'), value: historyMissions.length },
                { label: t('mission.successRate'), value: `${stats.rate}%` },
                { label: t('mission.totalTokens'), value: stats.totalTokens > 0 ? formatTokens(stats.totalTokens) : '—' },
              ]}
            />
          )}
          actions={(
            <div className="grid grid-cols-2 gap-1.5">
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={() => setMobileMissionBoardOpen(true)}>
                {t('mission.openBoard', '打开任务面板')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={() => setShowCreate(true)}>
                + {t('mission.create')}
              </Button>
              <Button variant="outline" className="h-9 justify-start rounded-[14px] px-3 text-[11px]" onClick={() => setMobileFilterSheetOpen(true)}>
                {t('mission.quickFilters', '筛选与视图')}
              </Button>
              <Button
                variant="outline"
                className="h-9 justify-start rounded-[14px] px-3 text-[11px]"
                onClick={() => navigate(`/teams/${teamId}?section=chat`)}
              >
                {t('mission.quickChatAssist', '回到对话继续处理')}
              </Button>
            </div>
          )}
          stage={(
            <div className="flex h-full min-h-[360px] flex-col gap-3 p-3">
              <div className="rounded-[18px] border border-border/65 bg-background/88 px-3.5 py-3">
                <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground">
                  {activeMission ? t('mission.currentMission', '当前活跃任务') : t('mission.executionContext', '执行上下文')}
                </div>
                <div className="mt-1 text-[13px] font-semibold text-foreground">
                  {activeMission ? activeMission.goal : t('mission.mobileBoardHeadline', '先进入任务面板，再查看队列、日志和产物。')}
                </div>
                <div className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                  {activeMission
                    ? `${activeMission.agent_name || t('mission.listHeaderAgent')} · ${t(`mission.${activeMission.status}`)}`
                    : t('mission.mobileBoardHint', '任务列表、历史和筛选都退到辅助面板，首屏只保留当前推进线索。')}
                </div>
              </div>
              <div className="grid grid-cols-2 gap-2">
                <Button
                  variant="outline"
                  className="h-9 justify-start rounded-[14px] px-3 text-[11px]"
                  onClick={() => setMobileMissionBoardOpen(true)}
                >
                  {t('mission.openBoard', '打开任务面板')}
                </Button>
                <Button
                  variant="outline"
                  className="h-9 justify-start rounded-[14px] px-3 text-[11px]"
                  onClick={() => {
                    if (activeMission) {
                      setSelectedMissionId(activeMission.mission_id);
                      setActiveTab('artifacts');
                    } else {
                      setMobileFilterSheetOpen(true);
                    }
                  }}
                >
                  {activeMission ? t('mission.openArtifactsWorkspace', '查看任务产物') : t('mission.quickFilters', '筛选与视图')}
                </Button>
              </div>
            </div>
          )}
          rail={(
            <ManagementRail
              title={t('mission.executionContext', '执行上下文')}
              description={t(
                'mission.executionContextListHint',
                '这里保留任务列表和真实状态，对话模式下优先围绕当前任务推进，再回来查看队列。',
              )}
            >
              <div className="space-y-2 rounded-[16px] border border-border/60 bg-background/82 px-3 py-3 text-[11px]">
                <div className="flex items-start justify-between gap-3">
                  <span className="text-muted-foreground">{t('mission.currentView', '当前视图')}</span>
                  <span className="text-right font-semibold text-foreground">
                    {t(`mission.view${viewMode === 'board' ? 'Board' : 'List'}`)}
                  </span>
                </div>
                <div className="flex items-start justify-between gap-3">
                  <span className="text-muted-foreground">{t('mission.filtered', '筛选结果')}</span>
                  <span className="text-right font-semibold text-foreground">{filteredMissions.length}</span>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  className="mt-1 h-8 w-full justify-center rounded-[12px] text-[11px]"
                  onClick={() => setMobileMissionBoardOpen(true)}
                >
                  {t('mission.openBoard', '打开任务面板')}
                </Button>
              </div>
            </ManagementRail>
          )}
          panel={(
            <>
              <BottomSheetPanel
                open={mobileMissionBoardOpen}
                onOpenChange={setMobileMissionBoardOpen}
                title={t('mission.openBoard', '打开任务面板')}
                description={t('mission.mobileBoardHint', '查看任务队列、历史记录、当前进度和产物入口。')}
                fullHeight
              >
                {mobileMissionBoardContent}
              </BottomSheetPanel>
              <BottomSheetPanel
                open={mobileFilterSheetOpen}
                onOpenChange={setMobileFilterSheetOpen}
                title={t('mission.quickFilters', '筛选与视图')}
                description={t('mission.filterHint', '调整任务搜索、Agent、历史状态和看板视图。')}
              >
                {missionFilterPanel}
              </BottomSheetPanel>
            </>
          )}
        />
        <CreateMissionDialog
          teamId={teamId}
          open={showCreate}
          onClose={() => setShowCreate(false)}
          onCreate={handleCreate}
        />
      </>
    );
  }

  return (
    <div className="flex flex-col h-[calc(100vh-40px)]">
      {/* Toolbar */}
      <div className="px-3 sm:px-5 py-3 border-b border-border/60">
        <div className="flex items-center gap-2 sm:gap-3 flex-wrap">
          <h2 className="text-base font-semibold tracking-tight shrink-0">{t('mission.title')}</h2>
          <div className="w-px h-5 bg-border/60 mx-1 hidden sm:block" />
          <div className="hidden sm:block flex-1 max-w-[240px]">
            <SearchInput
              placeholder={t('mission.searchMissions')}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onClear={() => setSearchQuery('')}
              className="h-8 text-xs !rounded-md"
            />
          </div>
          {agentNames.length > 1 && (
            <Select value={agentFilter} onValueChange={setAgentFilter}>
              <SelectTrigger className="hidden sm:flex w-36 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__all__">{t('mission.filterAgent')}</SelectItem>
                {agentNames.map(name => (
                  <SelectItem key={name} value={name}>{name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          {renderHistoryStatusFilter("hidden sm:flex w-32 h-8 text-xs")}
          <div className="ml-auto flex items-center gap-2 sm:gap-3">
            {missions.length > 0 && (
              <div className="hidden sm:flex items-center gap-2 text-caption text-muted-foreground">
                <span>{t('mission.successRate')} <span className="font-semibold tabular-nums text-foreground">{stats.rate}%</span></span>
                {stats.totalTokens > 0 && (
                  <>
                    <span className="text-border">·</span>
                    <span>{t('mission.totalTokens')} <span className="font-semibold tabular-nums text-foreground">{formatTokens(stats.totalTokens)}</span></span>
                  </>
                )}
              </div>
            )}
            <div className="hidden sm:flex rounded-md border border-border/60 overflow-hidden">
              {(['board', 'list'] as const).map(mode => (
                <button
                  key={mode}
                  onClick={() => setViewMode(mode)}
                  className={`px-3 py-1.5 text-caption uppercase tracking-wider transition-colors ${
                    viewMode === mode
                      ? 'bg-foreground/[0.06] font-semibold text-foreground'
                      : 'text-muted-foreground hover:text-foreground hover:bg-muted/40'
                  }`}
                >
                  {t(`mission.view${mode === 'board' ? 'Board' : 'List'}`)}
                </button>
              ))}
            </div>
            <Button size="sm" onClick={() => setShowCreate(true)}>
              + {t('mission.create')}
            </Button>
          </div>
        </div>
        {/* Mobile: search + filter row */}
        <div className="flex sm:hidden items-center gap-2 mt-2">
          <div className="flex-1">
            <SearchInput
              placeholder={t('mission.searchMissions')}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onClear={() => setSearchQuery('')}
              className="h-8 text-xs !rounded-md"
            />
          </div>
          {agentNames.length > 1 && (
            <Select value={agentFilter} onValueChange={setAgentFilter}>
              <SelectTrigger className="w-28 h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__all__">{t('mission.filterAgent')}</SelectItem>
                {agentNames.map(name => (
                  <SelectItem key={name} value={name}>{name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
          {renderHistoryStatusFilter("w-28 h-8 text-xs")}
        </div>
      </div>

      {boardBody}

      <CreateMissionDialog
        teamId={teamId}
        open={showCreate}
        onClose={() => setShowCreate(false)}
        onCreate={handleCreate}
      />
    </div>
  );
}

// ─── Mission Detail View (inline) ───

interface MissionDetailViewProps {
  teamId: string;
  mission: MissionDetail;
  monitorSnapshot: MissionMonitorSnapshot | null;
  missionId: string;
  messages: StreamMessage[];
  activeTab: DetailTab;
  setActiveTab: (tab: DetailTab) => void;
  startPending: boolean;
  onBack: () => void;
  onStart: () => void;
  onPause: () => void;
  onCancel: () => void;
  onDelete: () => void;
  onApproveStep: (idx: number, fb?: string) => void;
  onRejectStep: (idx: number, fb?: string) => void;
  onSkipStep: (idx: number) => void;
  onApproveGoal: (goalId: string) => void;
  onRejectGoal: (goalId: string) => void;
  onPivotGoal: (goalId: string) => void;
  onAbandonGoal: (goalId: string) => void;
}

function MissionDetailView({
  teamId, mission, monitorSnapshot, missionId, messages, activeTab, setActiveTab,
  startPending,
  onBack, onStart, onPause, onCancel, onDelete,
  onApproveStep, onRejectStep, onSkipStep,
  onApproveGoal, onRejectGoal, onPivotGoal, onAbandonGoal,
}: MissionDetailViewProps) {
  const { t } = useTranslation();

  const [selectedStepIndex, setSelectedStepIndex] = useState<number | null>(null);
  const [artifactCount, setArtifactCount] = useState<number | null>(null);
  const [showSummaryStrip, setShowSummaryStrip] = useState(true);
  const [showNavigationRail, setShowNavigationRail] = useState(true);
  const [showInsightPanel, setShowInsightPanel] = useState(true);

  const awaitingStep = mission.steps.find(s => s.status === 'awaiting_approval');
  const currentStep = mission.steps.find(s => s.index === mission.current_step);
  const currentGoal = mission.goal_tree?.find(g => g.goal_id === mission.current_goal_id)
    ?? mission.goal_tree?.find(g => g.status === 'running' || g.status === 'awaiting_approval' || g.status === 'pivoting')
    ?? mission.goal_tree?.find(g => g.status === 'pending')
    ?? null;
  const isAdaptive = isAdaptiveMissionDetail(mission);
  const isFinished = ['completed', 'failed', 'cancelled'].includes(mission.status);
  const displayStep = selectedStepIndex !== null
    ? mission.steps.find(s => s.index === selectedStepIndex) || currentStep
    : (isFinished ? null : currentStep);
  const completedSteps = mission.steps.filter(s => s.status === 'completed').length;
  const canStart =
    mission.status === 'draft' ||
    mission.status === 'planned' ||
    mission.status === 'paused' ||
    mission.status === 'failed';
  const canPause = mission.status === 'planning' || mission.status === 'running';
  const canCancelMission = ['draft', 'planned', 'planning', 'running', 'paused'].includes(mission.status);
  const canDelete = ['draft', 'cancelled', 'failed'].includes(mission.status);
  const isLive = ['planning', 'running'].includes(mission.status);

  useEffect(() => {
    if (isLive) {
      setSelectedStepIndex(null);
    }
  }, [isLive, mission.current_step, mission.current_run_id]);

  useEffect(() => {
    let cancelled = false;
    if (!missionId) {
      setArtifactCount(null);
      return;
    }
    missionApi.listArtifacts(missionId)
      .then(items => {
        if (!cancelled) {
          const nextItems = items || [];
          setArtifactCount(nextItems.length);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setArtifactCount(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [missionId, mission.updated_at]);

  // Elapsed timer
  const [elapsed, setElapsed] = useState('');
  useEffect(() => {
    if (!isLive) {
      setElapsed('');
      return;
    }
    const start = new Date(mission.started_at || mission.created_at).getTime();
    const tick = () => {
      const sec = Math.round((Date.now() - start) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [isLive, mission.started_at, mission.created_at]);

  // Stats from SSE messages + step data
  const { toolCalls, rounds } = useMemo(() => {
    let tc = 0, r = 0;
    let lastWasText = false;
    for (const m of messages) {
      if (m.type === 'toolcall') tc++;
      if (m.type === 'text') { if (!lastWasText) r++; lastWasText = true; }
      else lastWasText = false;
    }
    return { toolCalls: tc, rounds: r };
  }, [messages]);

  // Aggregate tool calls from step data (for finished missions)
  const stepToolCalls = useMemo(() =>
    mission.steps.reduce((sum, s) => sum + (s.tool_calls?.length ?? 0), 0),
  [mission.steps]);

  // Duration for finished missions
  const finishedDuration = useMemo(() => {
    if (!isFinished) return '';
    const start = new Date(mission.started_at || mission.created_at).getTime();
    const end = new Date(mission.completed_at || mission.updated_at).getTime();
    return formatDuration(end - start);
  }, [isFinished, mission.created_at, mission.updated_at]);

  const progressPct = mission.steps.length > 0
    ? Math.round((completedSteps / mission.steps.length) * 100) : 0;
  const latestSupervisorHint =
    monitorSnapshot?.step?.last_supervisor_hint ||
    displayStep?.last_supervisor_hint ||
    null;
  const latestBlocker =
    monitorSnapshot?.step?.current_blocker ||
    mission.error_message ||
    monitorSnapshot?.goal?.pivot_reason ||
    displayStep?.current_blocker ||
    currentGoal?.pivot_reason ||
    null;
  const currentVerification = displayStep?.contract_verification ?? currentGoal?.contract_verification;
  const evidenceTotals = useMemo(() => {
    return mission.steps.reduce((acc, step) => {
      const bundle = step.evidence_bundle;
      acc.artifacts += bundle?.artifact_paths?.length ?? 0;
      acc.requiredArtifacts += bundle?.required_artifact_paths?.length ?? 0;
      acc.planning += bundle?.planning_evidence_paths?.length ?? 0;
      acc.quality += bundle?.quality_evidence_paths?.length ?? 0;
      acc.runtime += bundle?.runtime_evidence_paths?.length ?? 0;
      acc.deployment += bundle?.deployment_evidence_paths?.length ?? 0;
      acc.review += bundle?.review_evidence_paths?.length ?? 0;
      acc.risk += bundle?.risk_evidence_paths?.length ?? 0;
      return acc;
    }, {
      artifacts: 0,
      requiredArtifacts: 0,
      planning: 0,
      quality: 0,
      runtime: 0,
      deployment: 0,
      review: 0,
      risk: 0,
    });
  }, [mission.steps]);
  const deliverySnapshot = useMemo(() => {
    const monitorStepBundle = monitorSnapshot?.step?.evidence_bundle;
    const observedSignals = new Set<string>([
      ...(monitorSnapshot?.step?.assessment?.observed_evidence ?? []),
      ...(monitorSnapshot?.goal?.assessment?.observed_evidence ?? []),
      ...(monitorSnapshot?.pending_intervention?.observed_evidence ?? []),
      ...(monitorSnapshot?.last_applied_intervention?.observed_evidence ?? []),
    ]);
    const requiredArtifacts = Math.max(
      evidenceTotals.requiredArtifacts,
      displayStep?.required_artifacts?.length ?? 0,
      currentGoal?.runtime_contract?.required_artifacts?.length ?? 0,
      monitorSnapshot?.step?.required_artifacts?.length ?? 0,
    );
    const runtimeEvidence = Math.max(
      evidenceTotals.runtime,
      monitorStepBundle?.runtime_evidence_paths?.length ?? 0,
      observedSignals.has('runtime_evidence_present') ? 1 : 0,
      monitorSnapshot?.goal?.has_runtime_contract ? 1 : 0,
      currentGoal?.runtime_contract ? 1 : 0,
    );
    const qualityEvidence = Math.max(
      evidenceTotals.quality,
      monitorStepBundle?.quality_evidence_paths?.length ?? 0,
      observedSignals.has('quality_evidence_present') ? 1 : 0,
      monitorSnapshot?.step?.assessment?.quality_summary ? 1 : 0,
      monitorSnapshot?.goal?.contract_verified ? 1 : 0,
      currentVerification?.accepted === true ? 1 : 0,
    );
    const planningEvidence = Math.max(
      evidenceTotals.planning,
      monitorStepBundle?.planning_evidence_paths?.length ?? 0,
      observedSignals.has('planning_evidence_present') ? 1 : 0,
      isAdaptive ? 1 : 0,
    );
    const riskEvidence = Math.max(
      evidenceTotals.risk,
      monitorStepBundle?.risk_evidence_paths?.length ?? 0,
      observedSignals.has('risk_evidence_present') ? 1 : 0,
      latestBlocker ? 1 : 0,
      monitorSnapshot?.step?.assessment?.risk_summary ? 1 : 0,
      monitorSnapshot?.goal?.assessment?.risk_summary ? 1 : 0,
    );

    return {
      artifacts: artifactCount ?? evidenceTotals.artifacts,
      requiredArtifacts,
      runtime: runtimeEvidence,
      quality: qualityEvidence,
      planning: planningEvidence,
      risk: riskEvidence,
    };
  }, [
    artifactCount,
    currentGoal,
    currentVerification,
    displayStep,
    evidenceTotals,
    isAdaptive,
    latestBlocker,
    monitorSnapshot,
  ]);
  const signalTitle = useMemo(() => {
    const statusAssessment =
      monitorSnapshot?.goal?.assessment?.status_assessment ||
      monitorSnapshot?.step?.assessment?.status_assessment;
    if (latestBlocker) return t('mission.attentionNeeded', 'Needs attention');
    if (statusAssessment === 'waiting_external') {
      return t('mission.waitingExternal', 'Waiting on external capacity');
    }
    if (statusAssessment === 'blocked') {
      return t('mission.attentionNeeded', 'Needs attention');
    }
    return t('mission.trackingNormally', 'Tracking normally');
  }, [latestBlocker, monitorSnapshot, t]);
  const signalBody = useMemo(() => {
    if (latestBlocker) return latestBlocker;
    const qualitySummary =
      monitorSnapshot?.step?.assessment?.quality_summary ||
      monitorSnapshot?.goal?.assessment?.quality_summary;
    if (qualitySummary) return qualitySummary;
    const riskSummary =
      monitorSnapshot?.step?.assessment?.risk_summary ||
      monitorSnapshot?.goal?.assessment?.risk_summary;
    if (riskSummary) return riskSummary;
    const observed = [
      ...(monitorSnapshot?.step?.assessment?.observed_evidence ?? []),
      ...(monitorSnapshot?.goal?.assessment?.observed_evidence ?? []),
    ];
    if (observed.length > 0) {
      return t('mission.signalObservedEvidence', {
        defaultValue: 'Observed signals: {{evidence}}',
        evidence: observed.slice(0, 3).join(', '),
      });
    }
    return t('mission.noCriticalBlockers', 'No critical blocker is exposed right now. The system is still tracking live work, evidence growth, and monitor interventions.');
  }, [latestBlocker, monitorSnapshot, t]);
  const currentFocusTitle = isAdaptive
    ? currentGoal?.title || t('mission.goalInProgress', 'Current goal')
    : displayStep?.title || t('mission.stepInProgress', 'Current step');
  const currentFocusDescription = isAdaptive
    ? currentGoal?.description || mission.context || ''
    : displayStep?.description || mission.context || '';
  const currentFocusSummary = isAdaptive
    ? currentGoal?.output_summary || ''
    : displayStep?.output_summary || '';
  const adaptiveStats = useMemo(() => {
    const goals = mission.goal_tree ?? [];
    return {
      completed: goals.filter(goal => goal.status === 'completed').length,
      active: goals.filter(goal => goal.status === 'running' || goal.status === 'awaiting_approval' || goal.status === 'pivoting').length,
      unresolved: goals.filter(goal => goal.status === 'pending' || goal.status === 'running' || goal.status === 'awaiting_approval' || goal.status === 'pivoting').length,
    };
  }, [mission.goal_tree]);
  const artifactsPrimary = activeTab === 'artifacts';
  const focusMode = artifactsPrimary && !showSummaryStrip && !showNavigationRail && !showInsightPanel;
  const layoutClass = artifactsPrimary
    ? (showNavigationRail ? 'xl:grid-cols-[296px_minmax(0,1fr)]' : 'xl:grid-cols-[minmax(0,1fr)]')
    : showNavigationRail
      ? (showInsightPanel ? 'xl:grid-cols-[296px_minmax(0,1fr)_320px]' : 'xl:grid-cols-[296px_minmax(0,1fr)]')
      : (showInsightPanel ? 'xl:grid-cols-[minmax(0,1fr)_320px]' : 'xl:grid-cols-[minmax(0,1fr)]');

  return (
    <div className="flex flex-col h-[calc(100vh-40px)] overflow-hidden">
      {/* Header */}
      <div className="px-3 sm:px-5 py-3 sm:py-4 border-b border-border/50">
        <div className="flex items-center gap-3 mb-2 sm:mb-3">
          <button onClick={onBack} className="text-sm text-muted-foreground hover:text-foreground transition-colors">
            ← {t('mission.title')}
          </button>
          <span className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
            {isLive && <span className="w-1.5 h-1.5 rounded-full bg-foreground/60 animate-pulse" />}
            {t(`mission.${mission.status}`)}
          </span>
        </div>

        <h1 className="text-sm sm:text-base font-semibold leading-snug">{mission.goal}</h1>

        {/* Progress bar */}
        {mission.steps.length > 0 && !artifactsPrimary && (
          <div className="mt-3 flex items-center gap-3">
            <span className="shrink-0 text-xs text-muted-foreground">
              Step {completedSteps}/{mission.steps.length}
            </span>
            <div className="h-1 flex-1 overflow-hidden rounded-full bg-muted">
              <div className="h-full rounded-full bg-foreground/30 transition-all duration-500" style={{ width: `${progressPct}%` }} />
            </div>
            <span className="text-xs text-muted-foreground/75">{progressPct}%</span>
          </div>
        )}

        {/* Metrics row */}
        <div className="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground">
          {/* Time */}
          {elapsed && (
            <span className="inline-flex items-center gap-1 font-mono tabular-nums">
              <span className="text-muted-foreground/55">⏱</span>
              <span className="font-semibold text-foreground">{elapsed}</span>
            </span>
          )}
          {finishedDuration && (
            <span className="inline-flex items-center gap-1">
              {t('mission.metricDuration')}: <span className="font-semibold tabular-nums text-foreground">{finishedDuration}</span>
            </span>
          )}
          {/* Tokens */}
          {mission.total_tokens_used > 0 && (
            <span className="inline-flex items-center gap-1">
              {t('mission.metricTokens')}: <span className="font-semibold tabular-nums text-foreground">{formatTokens(mission.total_tokens_used)}</span>
              {mission.token_budget > 0 && (
                <span className="text-muted-foreground/70">/ {formatTokens(mission.token_budget)}</span>
              )}
            </span>
          )}
          {/* Tool calls (live SSE or step aggregate) */}
          {(toolCalls > 0 || stepToolCalls > 0) && (
            <span className="inline-flex items-center gap-1">
              {t('mission.metricToolCalls')}: <span className="font-semibold tabular-nums text-foreground">{toolCalls || stepToolCalls}</span>
            </span>
          )}
          {/* Rounds */}
          {rounds > 0 && (
            <span className="inline-flex items-center gap-1">
              {t('mission.metricRounds')}: <span className="font-semibold tabular-nums text-foreground">{rounds}</span>
            </span>
          )}
          {/* Tags */}
          {!artifactsPrimary && (
            <>
              <span className="text-micro capitalize text-muted-foreground/72">{mission.approval_policy}</span>
              {isAdaptive && (
                <span className="text-micro text-status-info-text">AGE</span>
              )}
            </>
          )}
          {/* Adaptive stats */}
          {isAdaptive && mission.total_pivots > 0 && (
            <span className="inline-flex items-center gap-1">
              {t('mission.metricPivots')}: <span className="font-semibold tabular-nums text-foreground">{mission.total_pivots}</span>
            </span>
          )}
          {isAdaptive && mission.total_abandoned > 0 && (
            <span className="inline-flex items-center gap-1 text-status-warning-text">
              {t('mission.metricAbandoned')}: <span className="font-semibold tabular-nums">{mission.total_abandoned}</span>
            </span>
          )}
          {/* Attached docs */}
          {mission.attached_document_ids?.length > 0 && (
            <span className="inline-flex items-center gap-1">
              📎 <span className="font-semibold tabular-nums text-foreground">{mission.attached_document_ids.length}</span>
            </span>
          )}
        </div>

        {/* Context */}
        {mission.context && (
          <p className="mt-2 text-xs text-muted-foreground/70">{mission.context}</p>
        )}

        {/* Plan confirmation banner */}
        {mission.status === 'planned' && isAdaptive && mission.goal_tree && (
          <p className="text-xs text-muted-foreground mt-2">{t('mission.planReady')}</p>
        )}

        {/* Action buttons */}
          <div className="flex flex-wrap gap-2 mt-3">
            {canStart && (
              <button
                onClick={onStart}
                disabled={startPending}
                className="text-xs px-3 py-1.5 rounded-sm bg-foreground text-background hover:opacity-80 transition-opacity disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {(() => {
                  switch (mission.status) {
                    case 'paused': return t('mission.resume');
                    case 'failed': return t('mission.resumeFromFailed', 'Continue Failed Mission');
                    case 'planned': return isAdaptive
                      ? t('mission.confirmExecute') : t('mission.start');
                    default: return t('mission.start');
                  }
                })()}
            </button>
          )}
          {canPause && (
            <button onClick={onPause} className="text-xs px-3 py-1.5 rounded-sm border border-border hover:bg-accent transition-colors">
              {t('mission.pause')}
            </button>
          )}
          {canCancelMission && (
            <button onClick={onCancel} className="text-xs px-3 py-1.5 rounded-sm border border-border hover:bg-accent transition-colors">
              {t('mission.cancel')}
            </button>
          )}
          {canDelete && (
            <button onClick={onDelete} className="rounded-sm border border-[hsl(var(--status-error-text))/0.26] px-3 py-1.5 text-xs text-status-error-text transition-colors hover:bg-status-error-bg/70">
              {t('common.delete')}
            </button>
          )}
        </div>

        {mission.error_message && (
          <p className="mt-2 text-xs text-status-error-text/85">
            {localizeMissionError(mission.error_message, t)}
          </p>
        )}

        <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <PanelToggle
              active={focusMode}
              label={t('mission.layoutFocus', 'Focus')}
              onClick={() => {
                if (focusMode) {
                  setShowSummaryStrip(true);
                  setShowNavigationRail(true);
                  setShowInsightPanel(true);
                  return;
                }
                setActiveTab('artifacts');
                setShowSummaryStrip(false);
                setShowNavigationRail(false);
                setShowInsightPanel(false);
              }}
            />
            <PanelToggle
              active={showSummaryStrip}
              label={t('mission.layoutSummary', 'Summary')}
              onClick={() => setShowSummaryStrip(v => !v)}
            />
            <PanelToggle
              active={showNavigationRail}
              label={t('mission.layoutNavigation', 'Outline')}
              onClick={() => setShowNavigationRail(v => !v)}
            />
            {!artifactsPrimary && (
              <PanelToggle
                active={showInsightPanel}
                label={t('mission.layoutInsights', 'Insights')}
                onClick={() => setShowInsightPanel(v => !v)}
              />
            )}
          </div>
          <button
            onClick={() => setActiveTab('artifacts')}
            className="rounded-full bg-foreground px-4 py-2 text-xs font-medium text-background transition-opacity hover:opacity-88"
          >
            {t('mission.openArtifactsWorkspace', 'Open artifacts workspace')}
          </button>
        </div>

        {showSummaryStrip && (
          <div className="mt-3 overflow-hidden rounded-[22px] bg-[linear-gradient(180deg,rgba(251,248,242,0.92),rgba(255,255,255,0.98))] ring-1 ring-border/28">
            <div className="grid gap-px bg-border/14 sm:grid-cols-3">
              <div className="bg-transparent px-4 py-3.5">
                <SummaryBlock
                  label={t('mission.artifacts', 'Artifacts')}
                  value={artifactCount === null
                    ? t('mission.artifactHintLoading', 'Artifacts are loading...')
                    : String(artifactCount)}
                  note={artifactCount === null ? undefined : t('mission.deliveryAssetReady', 'Delivered assets ready for preview and reuse')}
                />
              </div>
              <div className="bg-transparent px-4 py-3.5">
                <SummaryBlock
                  label={t('mission.requiredArtifacts', 'Required')}
                  value={String(deliverySnapshot.requiredArtifacts)}
                  note={t('mission.coreDeliverables', 'Core deliverables still declared by the current contract')}
                />
              </div>
              <div className="bg-transparent px-4 py-3.5">
                <SummaryBlock
                  label={isAdaptive ? t('mission.goals', 'Goals') : t('mission.steps', 'Steps')}
                  value={isAdaptive
                    ? `${adaptiveStats.completed}/${mission.goal_tree?.length ?? 0}`
                    : `${completedSteps}/${mission.steps.length}`}
                  note={artifactsPrimary ? signalTitle : t('mission.progressOverview', 'Current completion progress')}
                />
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-hidden bg-[linear-gradient(180deg,rgba(248,244,236,0.48),rgba(255,255,255,0.78))] px-3 pb-4 sm:px-5 sm:pb-5">
      <div className={`grid h-full overflow-hidden rounded-[30px] bg-[linear-gradient(180deg,rgba(252,250,246,0.98),rgba(255,255,255,0.98))] shadow-[0_28px_72px_-56px_rgba(47,33,15,0.42)] ring-1 ring-border/32 ${layoutClass}`}>
        {/* Left: Steps/Goals */}
        {showNavigationRail && (
            <div className="flex w-full max-h-[40vh] flex-col overflow-hidden border-b border-border/20 bg-[linear-gradient(180deg,rgba(246,241,232,0.84),rgba(252,250,246,0.62))] shrink-0 xl:max-h-none xl:w-auto xl:border-b-0 xl:border-r xl:border-border/24">
            <div className="border-b border-border/18 px-5 py-4">
              <p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/56">
                {isAdaptive
                  ? t('mission.adaptiveOutline', 'Adaptive outline')
                  : t('mission.executionOutline', 'Execution outline')}
              </p>
              <div className="mt-2 flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 className="text-sm font-semibold text-foreground">
                    {isAdaptive
                      ? t('mission.goalStructure', 'Goal structure')
                      : t('mission.stepStructure', 'Step structure')}
                  </h2>
                  <p className="mt-1 text-xs leading-5 text-muted-foreground/72">
                    {isAdaptive
                      ? t('mission.goalStructureHint', 'Track which goal is current, which attempts already landed, and where the unresolved delivery work sits.')
                      : t('mission.stepStructureHint', 'Review execution order, the active checkpoint, and the remaining delivery work.')}
                  </p>
                </div>
                <div className="shrink-0 text-right">
                  <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground/54">
                    {isAdaptive
                      ? t('mission.completedGoals', 'Completed')
                      : t('mission.completedSteps', 'Completed')}
                  </p>
                  <p className="mt-1 text-sm font-semibold text-foreground">
                    {isAdaptive
                      ? `${adaptiveStats.completed}/${mission.goal_tree?.length ?? 0}`
                      : `${completedSteps}/${mission.steps.length}`}
                  </p>
                </div>
              </div>
              <div className="mt-3 grid grid-cols-2 overflow-hidden rounded-[18px] bg-background/74 text-[11px] ring-1 ring-border/18">
                <div className="px-3 py-2.5">
                  <p className="uppercase tracking-[0.14em] text-muted-foreground/52">
                    {t('mission.current', 'Current')}
                  </p>
                  <p className="mt-1 truncate font-medium text-foreground/88">
                    {isAdaptive
                      ? (currentGoal?.goal_id || t('mission.noneLabel', 'None'))
                      : (displayStep ? `Step ${displayStep.index + 1}` : t('mission.noneLabel', 'None'))}
                  </p>
                </div>
                <div className="border-l border-border/16 px-3 py-2.5">
                  <p className="uppercase tracking-[0.14em] text-muted-foreground/52">
                    {isAdaptive
                      ? t('mission.unresolved', 'Unresolved')
                      : t('mission.pending', 'Pending')}
                  </p>
                  <p className="mt-1 truncate font-medium text-foreground/88">
                    {isAdaptive
                      ? String(adaptiveStats.unresolved)
                      : String(Math.max(mission.steps.length - completedSteps, 0))}
                  </p>
                </div>
              </div>
            </div>
            <div className="min-h-0 overflow-y-auto px-4 py-4">
            {isAdaptive && mission.goal_tree ? (
              <GoalTreeView
                goals={mission.goal_tree}
                currentGoalId={mission.current_goal_id}
                onApprove={onApproveGoal}
                onReject={onRejectGoal}
                onPivot={onPivotGoal}
                onAbandon={onAbandonGoal}
              />
            ) : (
              <>
                {awaitingStep && (
                  <div className="mb-3">
                    <StepApprovalPanel
                      step={awaitingStep}
                      onApprove={(fb) => onApproveStep(awaitingStep.index, fb)}
                      onReject={(fb) => onRejectStep(awaitingStep.index, fb)}
                      onSkip={() => onSkipStep(awaitingStep.index)}
                    />
                  </div>
                )}
                <MissionStepList
                  steps={mission.steps}
                  currentStep={mission.current_step}
                  selectedStep={selectedStepIndex ?? undefined}
                  onSelectStep={setSelectedStepIndex}
                  onApprove={(idx) => onApproveStep(idx)}
                  onReject={(idx) => onRejectStep(idx)}
                  onSkip={(idx) => onSkipStep(idx)}
                />
              </>
            )}
            </div>
          </div>
        )}

        {/* Right: Tab content */}
        <div className={`min-h-0 flex flex-col overflow-hidden ${artifactsPrimary ? '' : 'border-r border-border/20'}`}>
          <div className="flex gap-6 border-b border-border/18 bg-transparent px-5 pt-3" role="tablist">
            {(['artifacts', 'work', 'evidence', 'logs'] as const).map(tab => (
              <button
                key={tab}
                role="tab"
                aria-selected={activeTab === tab}
                onClick={() => setActiveTab(tab)}
                className={`border-b px-0 pb-3 text-xs font-medium transition-colors ${
                  activeTab === tab
                    ? 'border-foreground/70 text-foreground'
                    : 'border-transparent text-muted-foreground/68 hover:border-border/42 hover:text-foreground'
                }`}
              >
                {{
                  work: t('mission.currentWork', 'Current work'),
                  artifacts: t('mission.artifacts', 'Artifacts'),
                  evidence: t('mission.evidence', 'Evidence'),
                  logs: t('mission.runtimeLogs', 'Runtime logs'),
                }[tab]}
              </button>
            ))}
          </div>

          <div className="flex-1 overflow-hidden">
            {activeTab === 'work' && (
              <MissionWorkSurface
                mission={mission}
                displayStep={displayStep}
                currentGoal={currentGoal}
                artifactCount={artifactCount}
                messages={selectedStepIndex === null ? messages : []}
                onSelectStep={setSelectedStepIndex}
                onSwitchToArtifacts={() => setActiveTab('artifacts')}
              />
            )}
            {activeTab === 'artifacts' && (
              <MissionArtifactsSurface
                missionId={missionId}
                teamId={teamId}
              />
            )}
            {activeTab === 'evidence' && (
              <MissionEvidenceSurface
                mission={mission}
                currentGoal={currentGoal}
                displayStep={displayStep}
                artifactCount={artifactCount}
                deliverySnapshot={deliverySnapshot}
                onSelectStep={setSelectedStepIndex}
              />
            )}
            {activeTab === 'logs' && (
              <MissionEventList
                missionId={missionId}
                isLive={isLive}
                runId={mission.current_run_id}
              />
            )}
          </div>
        </div>

        <aside className={!artifactsPrimary && showInsightPanel
          ? 'hidden min-h-0 overflow-y-auto bg-[linear-gradient(180deg,rgba(248,244,236,0.64),rgba(255,255,255,0.42))] px-4 py-4 xl:flex xl:flex-col xl:gap-3'
          : 'hidden'}>
          <InsightCard
            label={t('mission.currentWork', 'Current work')}
            title={currentFocusTitle}
          >
            {currentFocusDescription ? (
              <p className="text-sm leading-6 text-muted-foreground/82">
                {currentFocusDescription}
              </p>
            ) : (
              <p className="text-sm leading-6 text-muted-foreground/70">
                {t('mission.currentWorkFallback', 'The system is still producing work under the current mission focus.')}
              </p>
            )}
            <div className="mt-3 flex flex-wrap gap-2 text-xs">
              {displayStep?.supervisor_state && (
                <span className="rounded-full border border-border/70 bg-background/78 px-2.5 py-1 text-muted-foreground">
                  {t('mission.supervisor', 'Supervisor')}: {humanizeToken(displayStep.supervisor_state)}
                </span>
              )}
              {currentVerification?.status && (
                <span className="rounded-full border border-border/70 bg-background/78 px-2.5 py-1 text-muted-foreground">
                  {t('mission.verification', 'Verification')}: {humanizeToken(currentVerification.status)}
                </span>
              )}
            </div>
          </InsightCard>

          <InsightCard
            label={t('mission.signalPanel', 'Signal')}
            title={signalTitle}
            tone={latestBlocker ? 'warning' : 'neutral'}
          >
            <p className="text-sm leading-6 text-muted-foreground/82">
              {signalBody}
            </p>
            {latestSupervisorHint && latestSupervisorHint !== latestBlocker && (
              <p className="mt-3 rounded-xl border border-border/55 bg-background/72 px-3 py-2 text-xs leading-5 text-muted-foreground/78">
                {latestSupervisorHint}
              </p>
            )}
          </InsightCard>

          <InsightCard
            label={t('mission.deliverySnapshot', 'Delivery snapshot')}
            title={t('mission.coreDeliverySignals', 'Core delivery signals')}
          >
            <div className="grid grid-cols-2 gap-2 text-xs">
              <StatMini label={t('mission.artifacts', 'Artifacts')} value={String(deliverySnapshot.artifacts)} />
              <StatMini label={t('mission.requiredArtifacts', 'Required')} value={String(deliverySnapshot.requiredArtifacts)} />
              <StatMini label={t('mission.runtimeEvidence', 'Runtime')} value={String(deliverySnapshot.runtime)} />
              <StatMini label={t('mission.qualityEvidence', 'Quality')} value={String(deliverySnapshot.quality)} />
              <StatMini label={t('mission.planningEvidence', 'Planning')} value={String(deliverySnapshot.planning)} />
              <StatMini label={t('mission.riskEvidence', 'Risk')} value={String(deliverySnapshot.risk)} />
            </div>
          </InsightCard>

          {currentFocusSummary && (
            <InsightCard
              label={t('mission.latestOutcome', 'Latest outcome')}
              title={t('mission.latestOutcomeTitle', 'Most recent structured result')}
            >
              <MarkdownContent content={currentFocusSummary} className="text-sm" />
            </InsightCard>
          )}
        </aside>
      </div>
      </div>
    </div>
  );
}

// ─── Completion / Empty View ───

function statusIcon(status: string): string {
  switch (status) {
    case 'completed': return '✓';
    case 'failed': return '✗';
    case 'abandoned': return '⊘';
    case 'running': return '▶';
    case 'awaiting_approval': return '⏸';
    case 'pivoting': return '↻';
    default: return '○';
  }
}

function toneClasses(tone: 'neutral' | 'warning' | 'danger' = 'neutral'): string {
  switch (tone) {
    case 'warning':
      return 'ring-[hsl(var(--status-warning-text))/0.14] bg-status-warning-bg/70';
    case 'danger':
      return 'ring-[hsl(var(--status-error-text))/0.14] bg-status-error-bg/60';
    default:
      return 'ring-border/22 bg-background/82';
  }
}

function InsightCard({
  label,
  title,
  children,
  tone = 'neutral',
}: {
  label: string;
  title: string;
  children: ReactNode;
  tone?: 'neutral' | 'warning' | 'danger';
}) {
  return (
    <section className={`rounded-[20px] px-4 py-4 shadow-[0_14px_42px_-34px_rgba(47,33,15,0.32)] ring-1 ${toneClasses(tone)}`}>
      <p className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground/60">{label}</p>
      <h3 className="mt-2 text-sm font-semibold text-foreground">{title}</h3>
      <div className="mt-3">{children}</div>
    </section>
  );
}

function StatMini({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-[16px] bg-background/76 px-3 py-2 ring-1 ring-border/18">
      <p className="text-[11px] uppercase tracking-[0.12em] text-muted-foreground/55">{label}</p>
      <p className="mt-1 text-base font-semibold text-foreground">{value}</p>
    </div>
  );
}

function SummaryBlock({
  label,
  value,
  note,
}: {
  label: string;
  value: string;
  note?: string;
}) {
  return (
    <div className="min-w-0">
      <p className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground/56">{label}</p>
      <p className="mt-1 truncate text-sm font-semibold text-foreground">{value}</p>
      {note && (
        <p className="mt-1 line-clamp-2 text-xs leading-5 text-muted-foreground/72">{note}</p>
      )}
    </div>
  );
}

function PanelToggle({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`rounded-full px-3 py-1.5 text-xs font-medium transition-colors ${
        active
          ? 'bg-background text-foreground ring-1 ring-border/28'
          : 'bg-transparent text-muted-foreground ring-1 ring-transparent hover:bg-background/68 hover:text-foreground'
      }`}
    >
      {label}
    </button>
  );
}

function ContractPreview({
  requiredArtifacts,
  completionChecks,
  source,
}: {
  requiredArtifacts?: string[];
  completionChecks?: string[];
  source?: string;
}) {
  if ((!requiredArtifacts || requiredArtifacts.length === 0) && (!completionChecks || completionChecks.length === 0)) {
    return null;
  }

  const { t } = useTranslation();

  return (
    <div className="space-y-3 rounded-2xl border border-border/60 bg-background/82 px-4 py-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-[11px] uppercase tracking-[0.16em] text-muted-foreground/60">{t('mission.contract')}</p>
        {source && <span className="text-[11px] text-muted-foreground/55">{source}</span>}
      </div>
      {requiredArtifacts && requiredArtifacts.length > 0 && (
        <div>
          <p className="mb-2 text-xs font-medium text-foreground">{t('mission.requiredOutputs')}</p>
          <div className="flex flex-wrap gap-2">
            {requiredArtifacts.map((item) => (
              <span key={item} className="rounded-full border border-border/55 bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground">
                {item}
              </span>
            ))}
          </div>
        </div>
      )}
      {completionChecks && completionChecks.length > 0 && (
        <div>
          <p className="mb-2 text-xs font-medium text-foreground">{t('mission.checks')}</p>
          <div className="space-y-1.5">
            {completionChecks.slice(0, 4).map((check) => (
              <p key={check} className="rounded-xl border border-border/50 bg-muted/20 px-3 py-2 font-mono text-[11px] leading-5 text-muted-foreground/80">
                {check}
              </p>
            ))}
            {completionChecks.length > 4 && (
              <p className="text-xs text-muted-foreground/65">
                +{t('mission.moreChecks', { count: completionChecks.length - 4 })}
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function AdaptiveGoalWorkbench({
  goal,
}: {
  goal: GoalNode;
}) {
  const { t } = useTranslation();

  return (
    <div className="space-y-4 rounded-2xl border border-border/60 bg-background/88 px-4 py-4 shadow-[0_18px_52px_-40px_rgba(47,33,15,0.42)]">
      <div className="flex flex-wrap items-center gap-2">
        <span className="rounded-full border border-border/60 bg-muted/25 px-2.5 py-1 text-xs text-muted-foreground">
          {goal.goal_id}
        </span>
        <span className="rounded-full border border-border/60 bg-muted/25 px-2.5 py-1 text-xs text-muted-foreground">
          {goal.status}
        </span>
        {goal.is_checkpoint && (
          <span className="rounded-full border border-[hsl(var(--status-warning-text))/0.2] bg-status-warning-bg px-2.5 py-1 text-xs text-status-warning-text">
            {t('mission.checkpoint')}
          </span>
        )}
      </div>

      <div>
        <h3 className="text-base font-semibold leading-tight text-foreground">{goal.title}</h3>
        <p className="mt-2 text-sm leading-6 text-muted-foreground/82">{goal.description}</p>
      </div>

      <div className="rounded-2xl border border-border/60 bg-muted/18 px-4 py-3">
        <p className="mb-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/55">{t('mission.successCriteria')}</p>
        <p className="text-sm leading-6 text-muted-foreground/82">{goal.success_criteria}</p>
      </div>

      {goal.output_summary && (
        <div className="rounded-2xl border border-border/60 bg-background/78 px-4 py-3">
          <p className="mb-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/55">{t('mission.latestOutput')}</p>
          <MarkdownContent content={goal.output_summary} className="text-sm" />
        </div>
      )}

      <ContractPreview
        requiredArtifacts={goal.runtime_contract?.required_artifacts}
        completionChecks={goal.runtime_contract?.completion_checks}
        source={goal.runtime_contract?.source}
      />

      {goal.attempts.length > 0 && (
        <div className="rounded-2xl border border-border/60 bg-background/82 px-4 py-4">
          <p className="mb-3 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/55">{t('mission.recentAttempts')}</p>
          <div className="space-y-3">
            {goal.attempts.slice(-3).reverse().map((attempt) => (
              <div key={attempt.attempt_number} className="rounded-xl border border-border/55 bg-muted/20 px-3 py-3">
                <div className="flex items-center justify-between gap-3">
                  <span className="text-xs font-medium text-foreground">{t('mission.attemptLabel', { n: attempt.attempt_number })}</span>
                  <span className="text-xs text-muted-foreground">{attempt.signal}</span>
                </div>
                <p className="mt-2 text-sm text-muted-foreground/82">{attempt.approach}</p>
                {attempt.learnings && (
                  <p className="mt-2 text-xs leading-5 text-muted-foreground/72">{attempt.learnings}</p>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function MissionWorkSurface({
  mission,
  displayStep,
  currentGoal,
  artifactCount,
  messages,
  onSelectStep,
  onSwitchToArtifacts,
}: {
  mission: MissionDetail;
  displayStep: MissionStep | null | undefined;
  currentGoal: GoalNode | null;
  artifactCount: number | null;
  messages: StreamMessage[];
  onSelectStep: (idx: number) => void;
  onSwitchToArtifacts: () => void;
}) {
  const isAdaptive = isAdaptiveMissionDetail(mission);

  if (isAdaptive) {
    if (currentGoal) {
      return (
        <div className="h-full overflow-y-auto px-4 py-4">
          <AdaptiveGoalWorkbench goal={currentGoal} />
        </div>
      );
    }

    return (
      <CompletionView
        mission={mission}
        artifactCount={artifactCount}
        onSelectStep={onSelectStep}
        onSwitchToArtifacts={onSwitchToArtifacts}
      />
    );
  }

  if (displayStep) {
    return (
      <MissionStepDetail
        step={displayStep}
        isActive={mission.status === 'running' && mission.current_step === displayStep.index}
        messages={messages}
      />
    );
  }

  return (
    <CompletionView
      mission={mission}
      artifactCount={artifactCount}
      onSelectStep={onSelectStep}
      onSwitchToArtifacts={onSwitchToArtifacts}
    />
  );
}

function MissionArtifactsSurface({
  missionId,
  teamId,
}: {
  missionId: string;
  teamId: string;
}) {
  return (
    <div className="h-full overflow-hidden bg-[linear-gradient(180deg,rgba(250,247,241,0.35),rgba(255,255,255,0.9))]">
      <ArtifactList missionId={missionId} teamId={teamId} />
    </div>
  );
}

function MissionEvidenceSurface({
  mission,
  currentGoal,
  displayStep,
  artifactCount,
  deliverySnapshot,
  onSelectStep,
}: {
  mission: MissionDetail;
  currentGoal: GoalNode | null;
  displayStep: MissionStep | null | undefined;
  artifactCount: number | null;
  deliverySnapshot: {
    artifacts: number;
    planning: number;
    runtime: number;
    quality: number;
    risk: number;
  };
  onSelectStep: (idx: number) => void;
}) {
  const { t } = useTranslation();
  const evidenceTotals = mission.steps.reduce((acc, step) => {
    const bundle = step.evidence_bundle;
    acc.deployment += bundle?.deployment_evidence_paths?.length ?? 0;
    acc.review += bundle?.review_evidence_paths?.length ?? 0;
    return acc;
  }, { deployment: 0, review: 0 });
  const summaryTarget = displayStep?.output_summary || currentGoal?.output_summary || mission.final_summary || '';

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="overflow-y-auto px-4 py-4">
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1.05fr)_minmax(270px,0.95fr)]">
          <div className="space-y-4">
            {summaryTarget && (
              <div className="rounded-2xl border border-border/60 bg-background/86 px-4 py-4 shadow-[0_16px_42px_-34px_rgba(47,33,15,0.35)]">
                <p className="mb-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/55">
                  {t('mission.currentSummary', 'Current summary')}
                </p>
                <MarkdownContent content={summaryTarget} className="text-sm" />
              </div>
            )}

            {displayStep && (
              <ContractPreview
                requiredArtifacts={displayStep.required_artifacts}
                completionChecks={displayStep.completion_checks}
                source={displayStep.runtime_contract?.source}
              />
            )}

            {!displayStep && mission.steps.length > 0 && (
              <div className="rounded-2xl border border-border/60 bg-background/82 px-4 py-4">
                <p className="mb-3 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/55">
                  {t('mission.stepOutputs', 'Step outputs')}
                </p>
                <div className="space-y-2">
                  {mission.steps
                    .filter(step => step.output_summary)
                    .slice(-4)
                    .reverse()
                    .map(step => (
                      <button
                        key={step.index}
                        onClick={() => onSelectStep(step.index)}
                        className="w-full rounded-xl border border-border/55 bg-muted/18 px-3 py-3 text-left transition-colors hover:bg-accent"
                      >
                        <p className="text-xs font-medium text-foreground">
                          Step {step.index + 1}: {step.title}
                        </p>
                        <p className="mt-1 line-clamp-3 text-xs leading-5 text-muted-foreground/75">
                          {step.output_summary}
                        </p>
                      </button>
                    ))}
                </div>
              </div>
            )}
          </div>

          <div className="space-y-4">
            <InsightCard
              label={t('mission.evidenceBundle', 'Evidence bundle')}
              title={t('mission.deliveryCoverage', 'Delivery coverage')}
            >
              <div className="grid grid-cols-2 gap-2 text-xs">
                <StatMini label={t('mission.artifacts', 'Artifacts')} value={String(artifactCount ?? deliverySnapshot.artifacts)} />
                <StatMini label={t('mission.planningEvidence', 'Planning')} value={String(deliverySnapshot.planning)} />
                <StatMini label={t('mission.runtimeEvidence', 'Runtime')} value={String(deliverySnapshot.runtime)} />
                <StatMini label={t('mission.qualityEvidence', 'Quality')} value={String(deliverySnapshot.quality)} />
                <StatMini label={t('mission.deploymentEvidence', 'Deployment')} value={String(evidenceTotals.deployment)} />
                <StatMini label={t('mission.reviewEvidence', 'Review')} value={String(evidenceTotals.review)} />
              </div>
              {deliverySnapshot.risk > 0 && (
                <p className="mt-3 text-xs leading-5 text-status-warning-text">
                  {t('mission.riskEvidenceHint', { count: deliverySnapshot.risk })}
                </p>
              )}
            </InsightCard>
          </div>
        </div>
      </div>

    </div>
  );
}

function CompletionView({ mission, artifactCount, onSelectStep, onSwitchToArtifacts }: {
  mission: MissionDetail;
  artifactCount: number | null;
  onSelectStep: (idx: number) => void;
  onSwitchToArtifacts: () => void;
}) {
  const { t } = useTranslation();
  const isFinished = ['completed', 'failed', 'cancelled'].includes(mission.status);

  if (!isFinished) {
    return (
      <div className="flex h-full flex-col items-center justify-center text-muted-foreground/75">
        <span className="text-lg">◇</span>
        <span className="text-xs mt-1">
          {mission.status === 'draft' ? t('mission.draft') : t('mission.planning')}
        </span>
      </div>
    );
  }

  const isAdaptive = isAdaptiveMissionDetail(mission);
  const total = isAdaptive ? (mission.goal_tree?.length ?? 0) : mission.steps.length;
  const completed = isAdaptive
    ? (mission.goal_tree?.filter(g => g.status === 'completed').length ?? 0)
    : mission.steps.filter(s => s.status === 'completed').length;
  const failed = isAdaptive
    ? (mission.goal_tree?.filter(g => g.status === 'failed' || g.status === 'abandoned').length ?? 0)
    : mission.steps.filter(s => s.status === 'failed').length;
  const unresolved = Math.max(total - completed - failed, 0);

  return (
    <div className="flex flex-col h-full overflow-y-auto px-3 sm:px-6 py-4 sm:py-6">
      <div className="mb-5 rounded-3xl border border-border/60 bg-[linear-gradient(135deg,rgba(250,247,241,0.8),rgba(255,255,255,0.96))] px-5 py-5 shadow-[0_20px_54px_-42px_rgba(47,33,15,0.42)]">
        <div className="flex items-center gap-3">
          <span className="text-lg">{mission.status === 'completed' ? '✓' : '✗'}</span>
          <span className="text-sm font-semibold">
            {mission.status === 'completed' ? t('mission.missionComplete') : t('mission.missionFailed')}
          </span>
        </div>
        <p className="mt-3 text-sm leading-6 text-muted-foreground/78">
          {mission.status === 'completed' ? t('mission.missionComplete') : t('mission.missionFailed')}
        </p>
        <div className="mt-4 grid gap-2 sm:grid-cols-4">
          <StatMini label={isAdaptive ? t('mission.goals', 'Goals') : t('mission.steps', 'Steps')} value={String(total)} />
          <StatMini label={t('mission.completed', 'Completed')} value={String(completed)} />
          <StatMini label={t('mission.failed', 'Failed')} value={String(failed)} />
          <StatMini label={t('mission.pending', 'Pending')} value={String(unresolved)} />
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-3 text-xs text-muted-foreground/72">
          <span>
            {artifactCount === null
              ? t('mission.artifactHintLoading', 'Artifacts are loading...')
              : t('mission.artifactHint', { count: artifactCount })}
          </span>
          <button
            onClick={onSwitchToArtifacts}
            className="rounded-full border border-border px-3 py-1.5 transition-colors hover:bg-accent"
          >
            {t('mission.viewArtifacts', 'View artifacts')}
          </button>
        </div>
      </div>

      {mission.final_summary && (
        <div className="mb-5 rounded-2xl border border-border/60 bg-background/84 px-4 py-4">
          <p className="mb-2 text-[11px] uppercase tracking-[0.16em] text-muted-foreground/58">{t('mission.finalSummary', 'Final summary')}</p>
          <MarkdownContent content={mission.final_summary} className="text-sm" />
        </div>
      )}

      {isAdaptive ? (
        <div className="space-y-3">
          {mission.goal_tree?.map(goal => (
            <div
              key={goal.goal_id}
              className="w-full rounded-2xl border border-border/50 bg-background/78 px-4 py-3 text-left"
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-muted-foreground/70">
                  {statusIcon(goal.status)}
                </span>
                <span className="rounded-full border border-border/50 bg-muted/20 px-2 py-0.5 text-[11px] uppercase tracking-[0.12em] text-muted-foreground/72">{goal.goal_id}</span>
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{goal.title}</span>
                <span className="rounded-full border border-border/50 bg-muted/20 px-2 py-0.5 text-[11px] text-muted-foreground/72">{goal.status}</span>
              </div>
              {goal.output_summary && (
                <p className="mt-2 line-clamp-3 text-xs leading-5 text-muted-foreground/75">{goal.output_summary}</p>
              )}
            </div>
          ))}
        </div>
      ) : (
        <div className="space-y-3">
          {mission.steps.map(step => (
            <button
              key={step.index}
              onClick={() => onSelectStep(step.index)}
              className="w-full rounded-2xl border border-border/50 bg-background/78 px-4 py-3 text-left transition-colors hover:bg-accent"
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-muted-foreground/70">
                  {statusIcon(step.status)}
                </span>
                <span className="rounded-full border border-border/50 bg-muted/20 px-2 py-0.5 text-[11px] uppercase tracking-[0.12em] text-muted-foreground/72">
                  Step {step.index + 1}
                </span>
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{step.title}</span>
                <span className="rounded-full border border-border/50 bg-muted/20 px-2 py-0.5 text-[11px] text-muted-foreground/72">{step.status}</span>
              </div>
              {step.output_summary && (
                <p className="mt-2 line-clamp-3 text-xs leading-5 text-muted-foreground/75">{step.output_summary}</p>
              )}
            </button>
          ))}
        </div>
      )}

      {mission.error_message && (
        <p className="mt-4 text-xs text-status-error-text/85">
          {localizeMissionError(mission.error_message, t)}
        </p>
      )}
    </div>
  );
}
