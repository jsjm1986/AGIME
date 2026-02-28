import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useToast } from '../../contexts/ToastContext';
import { Button } from '../ui/button';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { SearchInput } from '../ui/search-input';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/select';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '../ui/table';
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
  MissionStatus,
  GoalStatus,
} from '../../api/mission';
import { ApiError } from '../../api/client';
import { localizeMissionError } from '../../utils/missionError';
import { formatDate } from '../../utils/format';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

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

const ACTIVE_STATUSES: MissionStatus[] = ['planning', 'planned', 'running'];
const HISTORY_STATUSES: MissionStatus[] = ['completed', 'paused', 'draft', 'failed', 'cancelled'];

interface MissionsPanelProps {
  teamId: string;
}

export function MissionsPanel({ teamId }: MissionsPanelProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();

  // Board state
  const [missions, setMissions] = useState<MissionListItem[]>([]);
  const [boardLoading, setBoardLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [agentFilter, setAgentFilter] = useState('__all__');
  const [historyStatusFilter, setHistoryStatusFilter] = useState<MissionStatus | '__all__'>('__all__');
  const [viewMode, setViewMode] = useState<'board' | 'list'>('board');

  // Detail state
  const [selectedMissionId, setSelectedMissionId] = useState<string | null>(null);
  const [mission, setMission] = useState<MissionDetail | null>(null);

  const [messages, setMessages] = useState<StreamMessage[]>([]);
  const [activeTab, setActiveTab] = useState<'output' | 'artifacts' | 'events'>('output');
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

  // Load mission detail
  const loadMission = useCallback(async () => {
    if (!selectedMissionId) return;
    try {
      const data = await missionApi.getMission(selectedMissionId);
      setMission(data);
    } catch (e) {
      console.error('Failed to load mission:', e);
    }
  }, [selectedMissionId]);

  useEffect(() => {
    if (selectedMissionId) {
      setMessages([]);
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

  // ─── Detail View ───
  if (selectedMissionId && mission) {
    return (
      <>
        <MissionDetailView
          teamId={teamId}
          mission={mission}
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

      {/* Content */}
      {boardLoading ? (
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
                const total = m.execution_mode === 'adaptive' ? m.goal_count : m.step_count;
                const done = m.execution_mode === 'adaptive' ? m.completed_goals : m.completed_steps;
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
                          <span className="text-caption tabular-nums text-muted-foreground/60">{pct}%</span>
                        </div>
                      )}
                    </TableCell>
                    <TableCell className="py-3 text-caption text-muted-foreground/50 tabular-nums hidden md:table-cell">{m.total_tokens_used > 0 ? formatTokens(m.total_tokens_used) : '—'}</TableCell>
                    <TableCell className="py-3 text-caption text-muted-foreground/50 tabular-nums hidden lg:table-cell">{formatDate(m.created_at)}</TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </div>
      ) : (
        <div className="flex-1 overflow-auto p-3 sm:p-4 space-y-4">
          {/* Active area */}
          {activeMissions.length > 0 ? (
            <div>
              <div className="flex items-center gap-2 mb-2">
                <h3 className="text-caption font-semibold uppercase tracking-wider text-muted-foreground/70">{t('mission.statsActive')}</h3>
                <span className="text-caption tabular-nums text-muted-foreground/50 bg-muted/60 px-1.5 py-0.5 rounded">
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
            <p className="text-[12px] text-muted-foreground/40 py-1">{t('mission.noActiveMissions')}</p>
          )}

          {/* Divider */}
          <div className="border-t border-border/40" />

          {/* History area */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <h3 className="text-caption font-semibold uppercase tracking-wider text-muted-foreground/70">{t('mission.history')}</h3>
              <span className="text-caption tabular-nums text-muted-foreground/50 bg-muted/60 px-1.5 py-0.5 rounded">
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
              <p className="text-[12px] text-muted-foreground/40">{t('mission.noMissions')}</p>
            )}
          </div>
        </div>
      )}

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
  missionId: string;
  messages: StreamMessage[];
  activeTab: 'output' | 'artifacts' | 'events';
  setActiveTab: (tab: 'output' | 'artifacts' | 'events') => void;
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
  teamId, mission, missionId, messages, activeTab, setActiveTab,
  startPending,
  onBack, onStart, onPause, onCancel, onDelete,
  onApproveStep, onRejectStep, onSkipStep,
  onApproveGoal, onRejectGoal, onPivotGoal, onAbandonGoal,
}: MissionDetailViewProps) {
  const { t } = useTranslation();

  const [selectedStepIndex, setSelectedStepIndex] = useState<number | null>(null);
  const [artifactCount, setArtifactCount] = useState<number | null>(null);

  const awaitingStep = mission.steps.find(s => s.status === 'awaiting_approval');
  const currentStep = mission.steps.find(s => s.index === mission.current_step);
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
  const effectiveExecutionProfile = mission.resolved_execution_profile ?? mission.execution_profile;
  let executionProfileLabel: string;
  switch (effectiveExecutionProfile) {
    case 'fast': executionProfileLabel = t('mission.profileFast', 'Fast'); break;
    case 'full': executionProfileLabel = t('mission.profileFull', 'Full'); break;
    default: executionProfileLabel = t('mission.profileAuto', 'Auto (Recommended)');
  }

  useEffect(() => {
    let cancelled = false;
    if (!isFinished) {
      setArtifactCount(null);
      return;
    }
    missionApi.listArtifacts(missionId)
      .then(items => {
        if (!cancelled) setArtifactCount(items?.length ?? 0);
      })
      .catch(() => {
        if (!cancelled) setArtifactCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [isFinished, missionId, mission.updated_at]);

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
        {mission.steps.length > 0 && (
          <div className="mt-3 flex items-center gap-3">
            <span className="text-xs text-muted-foreground shrink-0">
              Step {completedSteps}/{mission.steps.length}
            </span>
            <div className="flex-1 h-1 bg-muted rounded-full overflow-hidden">
              <div className="h-full bg-foreground/30 rounded-full transition-all duration-500" style={{ width: `${progressPct}%` }} />
            </div>
            <span className="text-xs text-muted-foreground/50">{progressPct}%</span>
          </div>
        )}

        {/* Metrics row */}
        <div className="flex items-center gap-2 sm:gap-3 flex-wrap mt-2.5 text-xs text-muted-foreground">
          {/* Time */}
          {elapsed && (
            <span className="inline-flex items-center gap-1 font-mono tabular-nums">
              <span className="text-muted-foreground/40">⏱</span>
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
                <span className="text-muted-foreground/50">/ {formatTokens(mission.token_budget)}</span>
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
          <span className="px-1.5 py-0.5 rounded bg-muted/60 text-micro capitalize">{mission.approval_policy}</span>
          <span className="px-1.5 py-0.5 rounded bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300 text-micro">
            {executionProfileLabel}
          </span>
          {mission.execution_mode === 'adaptive' && (
            <span className="px-1.5 py-0.5 rounded bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400 text-micro">AGE</span>
          )}
          {/* Adaptive stats */}
          {mission.execution_mode === 'adaptive' && mission.total_pivots > 0 && (
            <span className="inline-flex items-center gap-1">
              {t('mission.metricPivots')}: <span className="font-semibold tabular-nums text-foreground">{mission.total_pivots}</span>
            </span>
          )}
          {mission.execution_mode === 'adaptive' && mission.total_abandoned > 0 && (
            <span className="inline-flex items-center gap-1 text-amber-600 dark:text-amber-400">
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
          <p className="text-xs text-muted-foreground/50 mt-2">{mission.context}</p>
        )}

        {/* Plan confirmation banner */}
        {mission.status === 'planned' && mission.execution_mode === 'adaptive' && mission.goal_tree && (
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
                    case 'planned': return mission.execution_mode === 'adaptive'
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
            <button onClick={onDelete} className="text-xs px-3 py-1.5 rounded-sm border border-red-300 text-red-500 hover:bg-red-50 dark:hover:bg-red-950/20 transition-colors">
              {t('common.delete')}
            </button>
          )}
        </div>

        {mission.error_message && (
          <p className="text-xs text-red-400/80 mt-2">
            {localizeMissionError(mission.error_message, t)}
          </p>
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 flex flex-col sm:flex-row overflow-hidden">
        {/* Left: Steps/Goals */}
        <div className="w-full sm:w-72 max-h-[40vh] sm:max-h-none border-b sm:border-b-0 sm:border-r border-border/50 overflow-y-auto px-3 sm:px-4 py-2 sm:py-3 shrink-0">
          {mission.execution_mode === 'adaptive' && mission.goal_tree ? (
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

        {/* Right: Tab content */}
        <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
          <div className="flex border-b border-border/50" role="tablist">
            {(['output', 'artifacts', 'events'] as const).map(tab => (
              <button
                key={tab}
                role="tab"
                aria-selected={activeTab === tab}
                onClick={() => setActiveTab(tab)}
                className={`px-3 sm:px-4 py-2 text-xs transition-colors ${
                  activeTab === tab
                    ? 'border-b border-foreground/50 text-foreground'
                    : 'text-muted-foreground/50 hover:text-foreground'
                }`}
              >
                {{ output: t('mission.output'), artifacts: t('mission.artifacts'), events: t('mission.runtimeLogs', 'Runtime logs') }[tab]}
              </button>
            ))}
          </div>

          <div className="flex-1 overflow-hidden">
            {activeTab === 'output' && displayStep && (
              <MissionStepDetail
                step={displayStep}
                isActive={mission.status === 'running' && selectedStepIndex === null}
                messages={selectedStepIndex === null ? messages : []}
              />
            )}
            {activeTab === 'output' && !displayStep && (
              <CompletionView
                mission={mission}
                artifactCount={artifactCount}
                onSelectStep={setSelectedStepIndex}
                onSwitchToArtifacts={() => setActiveTab('artifacts')}
              />
            )}
            {activeTab === 'artifacts' && (
              <ArtifactList missionId={missionId} teamId={teamId} />
            )}
            {activeTab === 'events' && (
              <MissionEventList
                missionId={missionId}
                isLive={isLive}
                runId={mission.current_run_id}
              />
            )}
          </div>
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
      <div className="flex flex-col items-center justify-center h-full text-muted-foreground/40">
        <span className="text-lg">◇</span>
        <span className="text-xs mt-1">
          {mission.status === 'draft' ? t('mission.draft') : t('mission.planning')}
        </span>
      </div>
    );
  }

  const isAdaptive = mission.execution_mode === 'adaptive' && (mission.goal_tree?.length ?? 0) > 0;
  const total = isAdaptive ? (mission.goal_tree?.length ?? 0) : mission.steps.length;
  const completed = isAdaptive
    ? (mission.goal_tree?.filter(g => g.status === 'completed').length ?? 0)
    : mission.steps.filter(s => s.status === 'completed').length;
  const failed = isAdaptive
    ? (mission.goal_tree?.filter(g => g.status === 'failed' || g.status === 'abandoned').length ?? 0)
    : mission.steps.filter(s => s.status === 'failed').length;

  return (
    <div className="flex flex-col h-full overflow-y-auto px-3 sm:px-6 py-4 sm:py-6">
      <div className="flex items-center gap-3 mb-4">
        <span className="text-lg">{mission.status === 'completed' ? '✓' : '✗'}</span>
        <span className="text-sm font-semibold">
          {mission.status === 'completed' ? t('mission.missionComplete') : t('mission.missionFailed')}
        </span>
      </div>

      <p className="text-xs text-muted-foreground mb-5">
        {isAdaptive
          ? `${completed}/${total} ${t('mission.goals', 'goals')}`
          : t('mission.stepsCompleted', { completed, total: mission.steps.length })}
        {failed > 0 && <span className="text-red-400 ml-2">{failed} failed</span>}
      </p>

      <p className="text-xs text-muted-foreground/50 mb-4">
        {isAdaptive
          ? t('mission.reviewGoalsHint', 'Expand goals on the left to review details')
          : t('mission.selectStepToView')}
      </p>

      <div className="mb-4 rounded-md border border-border/50 bg-muted/20 px-3 py-2">
        <p className="text-xs text-muted-foreground">
          {artifactCount === null
            ? t('mission.artifactHintLoading', 'Artifacts are loading...')
            : t('mission.artifactHint', { count: artifactCount })}
        </p>
        <button
          onClick={onSwitchToArtifacts}
          className="mt-2 text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
        >
          {t('mission.viewArtifacts', 'View artifacts')}
        </button>
      </div>

      {mission.final_summary && (
        <div className="mb-4 rounded-md border border-border/50 bg-muted/20 px-3 py-2">
          <p className="text-xs text-muted-foreground mb-1">{t('mission.finalSummary', 'Final summary')}</p>
          <p className="text-xs leading-relaxed whitespace-pre-wrap">{mission.final_summary}</p>
        </div>
      )}

      {isAdaptive ? (
        <div className="space-y-2">
          {mission.goal_tree?.map(goal => (
            <div
              key={goal.goal_id}
              className="w-full text-left px-3 py-2 rounded-md border border-border/50"
            >
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground/50">
                  {statusIcon(goal.status)}
                </span>
                <span className="text-sm truncate">{goal.goal_id}: {goal.title}</span>
              </div>
              {goal.output_summary && (
                <p className="text-xs text-muted-foreground/60 mt-1 line-clamp-2">{goal.output_summary}</p>
              )}
            </div>
          ))}
        </div>
      ) : (
        <div className="space-y-2">
          {mission.steps.map(step => (
            <button
              key={step.index}
              onClick={() => onSelectStep(step.index)}
              className="w-full text-left px-3 py-2 rounded-md border border-border/50 hover:bg-accent transition-colors"
            >
              <div className="flex items-center gap-2">
                <span className="text-xs text-muted-foreground/50">
                  {statusIcon(step.status)}
                </span>
                <span className="text-sm truncate">{step.title}</span>
              </div>
              {step.output_summary && (
                <p className="text-xs text-muted-foreground/60 mt-1 line-clamp-2">{step.output_summary}</p>
              )}
            </button>
          ))}
        </div>
      )}

      {mission.error_message && (
        <p className="text-xs text-red-400/80 mt-4">
          {localizeMissionError(mission.error_message, t)}
        </p>
      )}
    </div>
  );
}
