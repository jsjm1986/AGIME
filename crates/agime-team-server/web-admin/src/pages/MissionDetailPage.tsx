import { useState, useEffect, useCallback, useRef } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Sidebar } from '../components/layout/Sidebar';
import { MissionStepList } from '../components/mission/MissionStepList';
import { MissionStepDetail } from '../components/mission/MissionStepDetail';
import { ArtifactList } from '../components/mission/ArtifactList';
import { MissionEventList } from '../components/mission/MissionEventList';
import { StepApprovalPanel } from '../components/mission/StepApprovalPanel';
import { GoalTreeView } from '../components/mission/GoalTreeView';
import {
  missionApi,
  MissionDetail,
  GoalStatus,
  GoalNode,
} from '../api/mission';
import { ApiError } from '../api/client';
import { ConfirmDialog } from '../components/ui/confirm-dialog';
import { StatusBadge, MISSION_STATUS_MAP } from '../components/ui/status-badge';
import { localizeMissionError } from '../utils/missionError';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

export default function MissionDetailPage() {
  const { teamId, missionId } = useParams<{ teamId: string; missionId: string }>();
  const navigate = useNavigate();
  const { t } = useTranslation();

  const [mission, setMission] = useState<MissionDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [messages, setMessages] = useState<StreamMessage[]>([]);
  const [activeTab, setActiveTab] = useState<'output' | 'artifacts' | 'events'>('output');
  const [toolCallCount, setToolCallCount] = useState(0);
  const [startPending, setStartPending] = useState(false);
  const [selectedStepIndex, setSelectedStepIndex] = useState<number | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);
  const lastEventIdRef = useRef<number | null>(null);
  const seenEventIdsRef = useRef<Set<number>>(new Set());
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef<number | null>(null);

  const resetStreamState = useCallback(() => {
    lastEventIdRef.current = null;
    seenEventIdsRef.current.clear();
    reconnectAttemptsRef.current = 0;
    if (reconnectTimerRef.current) {
      window.clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  // Load mission detail
  const loadMission = useCallback(async () => {
    if (!missionId) return;
    try {
      const data = await missionApi.getMission(missionId);
      setMission(data);
    } catch (e) {
      console.error('Failed to load mission:', e);
    } finally {
      setLoading(false);
    }
  }, [missionId]);

  useEffect(() => {
    resetStreamState();
    loadMission();
  }, [loadMission, resetStreamState]);

  // Auto-follow current step during live execution
  useEffect(() => {
    if (mission && ['planning', 'running'].includes(mission.status)) {
      setSelectedStepIndex(null);
    }
  }, [mission?.current_step, mission?.status]);

  // SSE streaming
  useEffect(() => {
    if (!missionId || !mission) return;
    const isLive = ['planning', 'running'].includes(mission.status);
    if (!isLive) return;

    let cancelled = false;

    const shouldHandleEvent = (e: Event) => {
      const raw = (e as MessageEvent).lastEventId;
      const parsed = Number(raw || 0);
      if (Number.isFinite(parsed) && parsed > 0) {
        const seen = seenEventIdsRef.current;
        if (seen.has(parsed)) return false;
        seen.add(parsed);
        if (seen.size > 10_000) {
          seen.clear();
          seen.add(parsed);
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

      const es = missionApi.streamMission(missionId, lastEventIdRef.current);
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

          for (let i = prev.length - 1, scanned = 0; i >= 0 && scanned < 80; i--, scanned++) {
            const item = prev[i];
            if (now - item.timestamp > 30_000) break;
            if (item.type !== type) continue;
            if (item.content.trim().replace(/\s+/g, ' ') === signature) {
              return prev;
            }
          }

          return [...prev, {
            type,
            content: raw,
            timestamp: now,
          }];
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
            resetStreamState();
            loadMission();
            es.close();
            eventSourceRef.current = null;
            return;
          }
          if (type === 'toolcall') {
            setToolCallCount(prev => prev + 1);
          }
          appendMessage(
            type,
            data.text || data.content || data.tool_name || JSON.stringify(data),
          );
        } catch {
          // ignore parse errors
        }
      };

      // ─── AGE goal event handlers: update goal tree in-place ───
      const makeGoalHandler = (
        eventType: string,
        updater: (data: Record<string, string>, prev: MissionDetail) => Partial<MissionDetail>,
        formatMsg: (data: Record<string, string>) => string,
      ) => (e: MessageEvent) => {
        if (!shouldHandleEvent(e)) return;
        try {
          const data = JSON.parse(e.data);
          setMission(prev => {
            if (!prev?.goal_tree) return prev;
            return { ...prev, ...updater(data, prev) };
          });
          appendMessage(eventType, formatMsg(data));
        } catch {
          // ignore parse errors
        }
      };

      const updateGoalStatus = (goalId: string, tree: MissionDetail['goal_tree'], patch: Partial<GoalNode>) =>
        tree!.map(g => g.goal_id === goalId ? { ...g, ...patch } : g);

      const handleGoalStart = makeGoalHandler(
        'goal_start',
        (data, prev) => ({
          current_goal_id: data.goal_id,
          goal_tree: updateGoalStatus(data.goal_id, prev.goal_tree, { status: 'running' as GoalStatus }),
        }),
        (data) => `▶ ${data.goal_id}: ${data.title}`,
      );

      const handleGoalComplete = makeGoalHandler(
        'goal_complete',
        (data, prev) => ({
          goal_tree: updateGoalStatus(data.goal_id, prev.goal_tree, { status: 'completed' as GoalStatus }),
        }),
        (data) => `✓ ${data.goal_id} (${data.signal})`,
      );

      const handlePivot = makeGoalHandler(
        'pivot',
        (data, prev) => ({
          total_pivots: prev.total_pivots + 1,
          goal_tree: updateGoalStatus(data.goal_id, prev.goal_tree, { status: 'pivoting' as GoalStatus, pivot_reason: data.to_approach }),
        }),
        (data) => `↻ ${data.goal_id}: ${data.from_approach} → ${data.to_approach}`,
      );

      const handleGoalAbandoned = makeGoalHandler(
        'goal_abandoned',
        (data, prev) => ({
          total_abandoned: prev.total_abandoned + 1,
          goal_tree: updateGoalStatus(data.goal_id, prev.goal_tree, { status: 'abandoned' as GoalStatus, pivot_reason: data.reason }),
        }),
        (data) => `⊘ ${data.goal_id}: ${data.reason}`,
      );

      es.addEventListener('text', handleEvent('text'));
      es.addEventListener('thinking', handleEvent('thinking'));
      es.addEventListener('toolcall', handleEvent('toolcall'));
      es.addEventListener('toolresult', handleEvent('toolresult'));
      es.addEventListener('status', handleEvent('status'));
      es.addEventListener('done', handleEvent('done'));
      // AGE goal events — real-time in-place updates
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
            const detail = await missionApi.getMission(missionId);
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
      resetStreamState();
    };
  }, [missionId, mission?.status, loadMission, resetStreamState]);

  // Action handlers
  const handleStart = async () => {
    if (!missionId || startPending) return;
    setStartPending(true);
    try {
      if (mission?.status === 'paused') {
        const res = await missionApi.resumeMission(missionId);
        if (res.status === 'pause_in_progress') {
          console.info('Pause is still draining; retry resume shortly');
        }
      } else if (mission?.status === 'failed') {
        const feedback = prompt(
          t(
            'mission.resumeFeedbackPrompt',
            'Optional: enter guidance for retry (leave empty to continue without extra guidance)',
          ),
        );
        await missionApi.resumeMission(missionId, feedback?.trim() || undefined);
      } else {
        await missionApi.startMission(missionId);
      }
      setMessages([]);
      setToolCallCount(0);
      resetStreamState();
      loadMission();
    } catch (e) {
      console.error('Failed to start mission:', e);
    } finally {
      setStartPending(false);
    }
  };

  const handlePause = async () => {
    if (!missionId) return;
    try {
      await missionApi.pauseMission(missionId);
      loadMission();
    } catch (e) {
      console.error('Failed to pause mission:', e);
    }
  };

  const handleCancel = async () => {
    if (!missionId) return;
    try {
      await missionApi.cancelMission(missionId);
      loadMission();
    } catch (e) {
      console.error('Failed to cancel mission:', e);
    }
  };

  const handleDelete = () => {
    if (!missionId || !teamId) return;
    setShowDeleteConfirm(true);
  };

  const confirmDelete = async () => {
    if (!missionId || !teamId) return;
    try {
      await missionApi.deleteMission(missionId);
      navigate(`/teams/${teamId}/missions`);
    } catch (e) {
      console.error('Failed to delete mission:', e);
    } finally {
      setShowDeleteConfirm(false);
    }
  };

  const handleApproveStep = async (stepIndex: number, feedback?: string) => {
    if (!missionId) return;
    try {
      await missionApi.approveStep(missionId, stepIndex, feedback);
      setMessages([]);
      loadMission();
    } catch (e) {
      console.error('Failed to approve step:', e);
    }
  };

  const handleRejectStep = async (stepIndex: number, feedback?: string) => {
    if (!missionId) return;
    try {
      await missionApi.rejectStep(missionId, stepIndex, feedback);
      loadMission();
    } catch (e) {
      console.error('Failed to reject step:', e);
    }
  };

  const handleSkipStep = async (stepIndex: number) => {
    if (!missionId) return;
    try {
      await missionApi.skipStep(missionId, stepIndex);
      loadMission();
    } catch (e) {
      console.error('Failed to skip step:', e);
    }
  };

  // ─── AGE Goal Handlers ───
  const handleApproveGoal = async (goalId: string) => {
    if (!missionId) return;
    try {
      await missionApi.approveGoal(missionId, goalId);
      setMessages([]);
      loadMission();
    } catch (e) {
      console.error('Failed to approve goal:', e);
    }
  };

  const handleRejectGoal = async (goalId: string) => {
    if (!missionId) return;
    try {
      await missionApi.rejectGoal(missionId, goalId);
      loadMission();
    } catch (e) {
      console.error('Failed to reject goal:', e);
    }
  };

  const handlePivotGoal = async (goalId: string) => {
    if (!missionId) return;
    const approach = prompt('Enter new approach:');
    if (!approach) return;
    try {
      await missionApi.pivotGoal(missionId, goalId, approach);
      loadMission();
    } catch (e) {
      console.error('Failed to pivot goal:', e);
    }
  };

  const handleAbandonGoal = async (goalId: string) => {
    if (!missionId) return;
    try {
      await missionApi.abandonGoal(missionId, goalId, 'Abandoned by user');
      loadMission();
    } catch (e) {
      console.error('Failed to abandon goal:', e);
    }
  };

  if (!teamId || !missionId) return null;

  if (loading) {
    return (
      <div className="flex h-screen">
        <Sidebar />
        <main className="flex-1 flex items-center justify-center">
          <p className="text-muted-foreground">Loading...</p>
        </main>
      </div>
    );
  }

  if (!mission) {
    return (
      <div className="flex h-screen">
        <Sidebar />
        <main className="flex-1 flex items-center justify-center">
          <p className="text-muted-foreground">Mission not found</p>
        </main>
      </div>
    );
  }

  const awaitingStep = mission.steps.find(s => s.status === 'awaiting_approval');
  const currentStep = mission.steps.find(s => s.index === mission.current_step);
  const isAdaptive = Boolean(mission.goal_tree && mission.goal_tree.length > 0);
  const isFinished = ['completed', 'failed', 'cancelled'].includes(mission.status);
  const displayStep = selectedStepIndex !== null
    ? mission.steps.find(s => s.index === selectedStepIndex) || currentStep
    : (isFinished ? null : currentStep);
  const isDisplayStepActive = displayStep != null && currentStep != null && displayStep.index === currentStep.index && mission.status === 'running';
  const completedSteps = mission.steps.filter(s => s.status === 'completed').length;
  const canStart =
    mission.status === 'draft' ||
    mission.status === 'planned' ||
    mission.status === 'paused' ||
    mission.status === 'failed';
  const isLive = ['planning', 'running'].includes(mission.status);
  const canPause = mission.status === 'planning' || mission.status === 'running';
  const canCancel = ['draft', 'planned', 'planning', 'running', 'paused'].includes(mission.status);
  const canDelete = ['draft', 'cancelled', 'failed'].includes(mission.status);
  return (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex-1 overflow-hidden flex flex-col">
        {/* Header */}
        <div className="p-4 border-b">
          <div className="flex items-center gap-3 mb-2">
            <button
              onClick={() => navigate(`/teams/${teamId}/missions`)}
              className="text-sm text-muted-foreground hover:text-foreground"
            >
              &larr; {t('mission.title')}
            </button>
            <StatusBadge status={MISSION_STATUS_MAP[mission.status]}>
              {t(`mission.${mission.status}`)}
            </StatusBadge>
          </div>
          <h1 className="text-lg font-semibold line-clamp-2">{mission.goal}</h1>
          <div className="flex items-center gap-4 mt-2 text-xs text-muted-foreground">
            {isLive && toolCallCount > 0 && (
              <span>{t('mission.toolCalls', { count: toolCallCount })}</span>
            )}
            {isAdaptive && mission.goal_tree ? (
              <>
                <span>{t('mission.goals')}: {mission.goal_tree.filter(g => g.status === 'completed').length}/{mission.goal_tree.length}</span>
                {mission.total_pivots > 0 && <span>{t('mission.pivots')}: {mission.total_pivots}</span>}
                {mission.total_abandoned > 0 && <span>{t('mission.abandonedCount')}: {mission.total_abandoned}</span>}
              </>
            ) : (
              <span>{t('mission.progress', { completed: completedSteps, total: mission.steps.length })}</span>
            )}
            <span className="capitalize">{mission.approval_policy}</span>
          </div>

          {/* Plan confirmation banner */}
          {mission.status === 'planned' && isAdaptive && mission.goal_tree && (
            <div className="mt-2 rounded p-2 text-sm border border-[hsl(var(--status-info-text))/0.16] bg-[hsl(var(--status-info-bg))/0.72] text-status-info-text">
              {t('mission.planReady')}
            </div>
          )}

          {/* Action buttons */}
          <div className="flex gap-2 mt-3">
            {canStart && (
              <button
                onClick={handleStart}
                disabled={startPending}
                className="px-3 py-1.5 text-sm rounded-md bg-primary text-primary-foreground hover:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed"
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
              <button onClick={handlePause} className="px-3 py-1.5 text-sm rounded-md border border-[hsl(var(--status-warning-text))/0.2] bg-status-warning-bg text-status-warning-text hover:opacity-90">
                {t('mission.pause')}
              </button>
            )}
            {canCancel && (
              <button onClick={handleCancel} className="px-3 py-1.5 text-sm rounded-md border hover:bg-accent">
                {t('mission.cancel', 'Cancel')}
              </button>
            )}
            {canDelete && (
              <button onClick={handleDelete} className="px-3 py-1.5 text-sm rounded-md border border-[hsl(var(--status-error-text))/0.18] text-status-error-text hover:bg-[hsl(var(--status-error-bg))/0.72]">
                {t('common.delete', 'Delete')}
              </button>
            )}
          </div>

          {mission.error_message && (
            <div className="mt-2 rounded p-2 text-sm border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.72] text-status-error-text">
              {localizeMissionError(mission.error_message, t)}
            </div>
          )}
        </div>

        {/* Content area */}
        <div className="flex-1 flex overflow-hidden">
          {/* Left: Steps */}
          <div className="w-80 border-r overflow-y-auto p-3">
            {isAdaptive && mission.goal_tree ? (
              <GoalTreeView
                goals={mission.goal_tree}
                currentGoalId={mission.current_goal_id}
                onApprove={handleApproveGoal}
                onReject={handleRejectGoal}
                onPivot={handlePivotGoal}
                onAbandon={handleAbandonGoal}
              />
            ) : (
              <>
                {awaitingStep && (
                  <div className="mb-3">
                    <StepApprovalPanel
                      step={awaitingStep}
                      onApprove={(fb) => handleApproveStep(awaitingStep.index, fb)}
                      onReject={(fb) => handleRejectStep(awaitingStep.index, fb)}
                      onSkip={() => handleSkipStep(awaitingStep.index)}
                    />
                  </div>
                )}
                <MissionStepList
                  steps={mission.steps}
                  currentStep={mission.current_step}
                  selectedStep={selectedStepIndex ?? mission.current_step}
                  onSelectStep={(idx) => setSelectedStepIndex(idx)}
                  onApprove={(idx) => handleApproveStep(idx)}
                  onReject={(idx) => handleRejectStep(idx)}
                  onSkip={(idx) => handleSkipStep(idx)}
                />
              </>
            )}
          </div>

          {/* Right: Tab content */}
          <div className="flex-1 flex flex-col overflow-hidden">
            {/* Tabs */}
            <div className="flex border-b">
              {(['output', 'artifacts', 'events'] as const).map(tab => (
                <button
                  key={tab}
                  onClick={() => setActiveTab(tab)}
                  className={`px-4 py-2 text-sm border-b-2 transition-colors ${
                    activeTab === tab
                      ? 'border-primary text-foreground'
                      : 'border-transparent text-muted-foreground hover:text-foreground'
                  }`}
                >
                  {{ output: t('mission.output'), artifacts: t('mission.artifacts'), events: t('mission.runtimeLogs', 'Runtime logs') }[tab]}
                </button>
              ))}
            </div>

            {/* Tab content */}
            <div className="flex-1 overflow-hidden">
              {activeTab === 'output' && displayStep && (
                <MissionStepDetail
                  step={displayStep}
                  isActive={isDisplayStepActive}
                  messages={isDisplayStepActive ? messages : []}
                />
              )}
              {activeTab === 'output' && !displayStep && (
                <div className="flex flex-col items-center justify-center h-full text-muted-foreground text-sm px-6 text-center">
                  <p>
                    {({ completed: t('mission.completed'), draft: t('mission.draft') } as Record<string, string>)[mission.status] ?? t('mission.planning')}
                  </p>
                  {mission.final_summary && (
                    <p className="mt-3 max-w-2xl whitespace-pre-wrap leading-relaxed text-xs text-muted-foreground/80">
                      {mission.final_summary}
                    </p>
                  )}
                </div>
              )}
              {activeTab === 'artifacts' && (
                <ArtifactList missionId={missionId} teamId={teamId || ''} />
              )}
              {activeTab === 'events' && (
                <MissionEventList missionId={missionId} isLive={isLive} />
              )}
            </div>
          </div>
        </div>
      </main>
      <ConfirmDialog
        open={showDeleteConfirm}
        onOpenChange={setShowDeleteConfirm}
        title={t('mission.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDelete}
      />
    </div>
  );
}
