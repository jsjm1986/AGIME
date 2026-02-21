import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { MissionCard } from '../mission/MissionCard';
import { CreateMissionDialog } from '../mission/CreateMissionDialog';
import { MissionStepList } from '../mission/MissionStepList';
import { MissionStepDetail } from '../mission/MissionStepDetail';
import { ArtifactList } from '../mission/ArtifactList';
import { StepApprovalPanel } from '../mission/StepApprovalPanel';
import { GoalTreeView } from '../mission/GoalTreeView';
import {
  missionApi,
  MissionDetail,
  MissionListItem,
  MissionStatus,
  GoalStatus,
} from '../../api/mission';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

type BoardColumn = {
  key: string;
  statuses: MissionStatus[];
  labelKey: string;
};

const columns: BoardColumn[] = [
  { key: 'active', statuses: ['planning', 'planned', 'running'], labelKey: 'mission.running' },
  { key: 'paused', statuses: ['paused'], labelKey: 'mission.paused' },
  { key: 'completed', statuses: ['completed'], labelKey: 'mission.completed' },
  { key: 'other', statuses: ['draft', 'failed', 'cancelled'], labelKey: 'mission.draft' },
];


interface MissionsPanelProps {
  teamId: string;
}

export function MissionsPanel({ teamId }: MissionsPanelProps) {
  const { t } = useTranslation();

  // Board state
  const [missions, setMissions] = useState<MissionListItem[]>([]);
  const [boardLoading, setBoardLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);

  // Detail state
  const [selectedMissionId, setSelectedMissionId] = useState<string | null>(null);
  const [mission, setMission] = useState<MissionDetail | null>(null);

  const [messages, setMessages] = useState<StreamMessage[]>([]);
  const [activeTab, setActiveTab] = useState<'output' | 'artifacts'>('output');

  // Dialog state
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false);
  const [pivotDialogOpen, setPivotDialogOpen] = useState(false);
  const [pivotGoalId, setPivotGoalId] = useState<string | null>(null);
  const [pivotApproach, setPivotApproach] = useState('');
  const eventSourceRef = useRef<EventSource | null>(null);

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
      loadMission();
    }
  }, [selectedMissionId, loadMission]);

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

    const es = missionApi.streamMission(selectedMissionId);
    eventSourceRef.current = es;

    const handleEvent = (type: string) => (e: MessageEvent) => {
      try {
        const data = JSON.parse(e.data);
        if (type === 'status') { loadMission(); return; }
        if (type === 'done') { loadMission(); es.close(); return; }
        setMessages(prev => [...prev, {
          type,
          content: data.text || data.content || data.tool_name || JSON.stringify(data),
          timestamp: Date.now(),
        }]);
      } catch { /* ignore */ }
    };

    const handleGoalStart = (e: MessageEvent) => {
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
        setMessages(prev => [...prev, {
          type: 'goal_start',
          content: `▶ ${data.goal_id}: ${data.title}`,
          timestamp: Date.now(),
        }]);
      } catch { /* ignore */ }
    };

    const handleGoalComplete = (e: MessageEvent) => {
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
        setMessages(prev => [...prev, {
          type: 'goal_complete',
          content: `✓ ${data.goal_id} (${data.signal})`,
          timestamp: Date.now(),
        }]);
      } catch { /* ignore */ }
    };

    const handlePivot = (e: MessageEvent) => {
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
        setMessages(prev => [...prev, {
          type: 'pivot',
          content: `↻ ${data.goal_id}: ${data.from_approach} → ${data.to_approach}`,
          timestamp: Date.now(),
        }]);
      } catch { /* ignore */ }
    };

    const handleGoalAbandoned = (e: MessageEvent) => {
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
        setMessages(prev => [...prev, {
          type: 'goal_abandoned',
          content: `⊘ ${data.goal_id}: ${data.reason}`,
          timestamp: Date.now(),
        }]);
      } catch { /* ignore */ }
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
    es.onerror = () => { es.close(); loadMission(); };

    return () => {
      es.close();
      eventSourceRef.current = null;
    };
  }, [selectedMissionId, mission?.status]);

  // Action handlers
  const handleStart = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.startMission(selectedMissionId);
      setMessages([]);
    } catch (e) { console.error('Failed to start mission:', e); }
    loadMission();
  };

  const handlePauseMission = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.pauseMission(selectedMissionId);
      loadMission();
    } catch (e) { console.error('Failed to pause mission:', e); }
  };

  const handleCancelMission = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.cancelMission(selectedMissionId);
      loadMission();
    } catch (e) { console.error('Failed to cancel mission:', e); }
  };

  const handleDeleteMission = async () => {
    if (!selectedMissionId) return;
    try {
      await missionApi.deleteMission(selectedMissionId);
      setSelectedMissionId(null);
      setMission(null);
      setDeleteConfirmOpen(false);
      loadMissions();
    } catch (e) { console.error('Failed to delete mission:', e); }
  };

  const handleApproveStep = async (stepIndex: number, feedback?: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.approveStep(selectedMissionId, stepIndex, feedback);
      setMessages([]);
      loadMission();
    } catch (e) { console.error('Failed to approve step:', e); }
  };

  const handleRejectStep = async (stepIndex: number, feedback?: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.rejectStep(selectedMissionId, stepIndex, feedback);
      loadMission();
    } catch (e) { console.error('Failed to reject step:', e); }
  };

  const handleSkipStep = async (stepIndex: number) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.skipStep(selectedMissionId, stepIndex);
      loadMission();
    } catch (e) { console.error('Failed to skip step:', e); }
  };

  const handleApproveGoal = async (goalId: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.approveGoal(selectedMissionId, goalId);
      setMessages([]);
      loadMission();
    } catch (e) { console.error('Failed to approve goal:', e); }
  };

  const handleRejectGoal = async (goalId: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.rejectGoal(selectedMissionId, goalId);
      loadMission();
    } catch (e) { console.error('Failed to reject goal:', e); }
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
    } catch (e) { console.error('Failed to pivot goal:', e); }
  };

  const handleAbandonGoal = async (goalId: string) => {
    if (!selectedMissionId) return;
    try {
      await missionApi.abandonGoal(selectedMissionId, goalId, t('mission.abandonedByUser'));
      loadMission();
    } catch (e) { console.error('Failed to abandon goal:', e); }
  };

  const handleCreate = async (data: Parameters<typeof missionApi.createMission>[0]) => {
    try {
      await missionApi.createMission(data);
      setShowCreate(false);
      loadMissions();
    } catch (e) { console.error('Failed to create mission:', e); }
  };

  const handleBack = () => {
    setSelectedMissionId(null);
    setMission(null);
    setMessages([]);
    loadMissions();
  };

  // ─── Detail View ───
  if (selectedMissionId && mission) {
    return (
      <>
        <MissionDetailView
          mission={mission}
          missionId={selectedMissionId}
          messages={messages}
          activeTab={activeTab}
          setActiveTab={setActiveTab}
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
      </>
    );
  }

  // ─── Board View ───
  const grouped = columns.map(col => ({
    ...col,
    items: missions.filter(m => col.statuses.includes(m.status)),
  }));

  return (
    <div className="flex flex-col h-[calc(100vh-40px)]">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b">
        <h2 className="text-lg font-semibold">{t('mission.title')}</h2>
        <Button size="sm" onClick={() => setShowCreate(true)}>
          + {t('mission.create')}
        </Button>
      </div>

      {/* Board */}
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
      ) : (
        <div className="flex-1 overflow-x-auto p-4">
          <div className="flex gap-4 h-full min-w-max">
            {grouped.map(col => (
              <div key={col.key} className="w-72 flex flex-col">
                <div className="flex items-center gap-2 mb-3">
                  <h3 className="text-sm font-semibold">{t(col.labelKey)}</h3>
                  <span className="text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded-full">
                    {col.items.length}
                  </span>
                </div>
                <div className="flex-1 space-y-2 overflow-y-auto">
                  {col.items.map(m => (
                    <MissionCard
                      key={m.mission_id}
                      mission={m}
                      onClick={(id) => setSelectedMissionId(id)}
                    />
                  ))}
                </div>
              </div>
            ))}
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
  mission: MissionDetail;
  missionId: string;
  messages: StreamMessage[];
  activeTab: 'output' | 'artifacts';
  setActiveTab: (tab: 'output' | 'artifacts') => void;
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
  mission, missionId, messages, activeTab, setActiveTab,
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
  const canStart = mission.status === 'draft' || mission.status === 'planned';
  const canPause = mission.status === 'running';
  const canCancelMission = ['planning', 'running', 'paused'].includes(mission.status);
  const canDelete = ['draft', 'cancelled', 'failed'].includes(mission.status);
  const isLive = ['planning', 'running'].includes(mission.status);

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
    const start = new Date(mission.created_at).getTime();
    const tick = () => {
      const sec = Math.round((Date.now() - start) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [isLive, mission.created_at]);

  // Stats from SSE messages
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

  const progressPct = mission.steps.length > 0
    ? Math.round((completedSteps / mission.steps.length) * 100) : 0;

  return (
    <div className="flex flex-col h-[calc(100vh-40px)] overflow-hidden">
      {/* Header */}
      <div className="px-5 py-4 border-b border-border/50">
        <div className="flex items-center gap-3 mb-3">
          <button onClick={onBack} className="text-sm text-muted-foreground hover:text-foreground transition-colors">
            ← {t('mission.title')}
          </button>
          <span className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
            {isLive && <span className="w-1.5 h-1.5 rounded-full bg-foreground/60 animate-pulse" />}
            {t(`mission.${mission.status}`)}
          </span>
        </div>

        <h1 className="text-base font-semibold leading-snug">{mission.goal}</h1>

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
        <div className="flex items-center gap-5 mt-3 text-xs text-muted-foreground">
          {elapsed && <span className="font-mono">{elapsed}</span>}
          {toolCalls > 0 && <span>↗ {toolCalls} calls</span>}
          {rounds > 0 && <span>◎ {rounds} rounds</span>}
          <span className="capitalize">{mission.approval_policy}</span>
          {mission.execution_mode === 'adaptive' && <span>adaptive</span>}
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
        <div className="flex gap-2 mt-3">
          {canStart && (
            <button onClick={onStart} className="text-xs px-3 py-1.5 rounded-sm bg-foreground text-background hover:opacity-80 transition-opacity">
              {mission.status === 'planned' && mission.execution_mode === 'adaptive'
                ? t('mission.confirmExecute')
                : t('mission.start')}
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
          <p className="text-xs text-red-400/80 mt-2">{mission.error_message}</p>
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 flex overflow-hidden">
        {/* Left: Steps/Goals */}
        <div className="w-72 border-r border-border/50 overflow-y-auto px-4 py-3">
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
        <div className="flex-1 flex flex-col overflow-hidden">
          <div className="flex border-b border-border/50" role="tablist">
            {(['output', 'artifacts'] as const).map(tab => (
              <button
                key={tab}
                role="tab"
                aria-selected={activeTab === tab}
                onClick={() => setActiveTab(tab)}
                className={`px-4 py-2 text-xs transition-colors ${
                  activeTab === tab
                    ? 'border-b border-foreground/50 text-foreground'
                    : 'text-muted-foreground/50 hover:text-foreground'
                }`}
              >
                {tab === 'output' ? t('mission.output') : t('mission.artifacts')}
              </button>
            ))}
          </div>

          <div className="flex-1 overflow-hidden">
            {activeTab === 'output' && displayStep && (
              <MissionStepDetail
                step={displayStep}
                missionId={missionId}
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
              <ArtifactList missionId={missionId} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Completion / Empty View ───

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

  const completed = mission.steps.filter(s => s.status === 'completed').length;
  const failed = mission.steps.filter(s => s.status === 'failed').length;

  return (
    <div className="flex flex-col h-full overflow-y-auto px-6 py-6">
      <div className="flex items-center gap-3 mb-4">
        <span className="text-lg">{mission.status === 'completed' ? '✓' : '✗'}</span>
        <span className="text-sm font-semibold">
          {mission.status === 'completed' ? t('mission.missionComplete') : t('mission.missionFailed')}
        </span>
      </div>

      <p className="text-xs text-muted-foreground mb-5">
        {t('mission.stepsCompleted', { completed, total: mission.steps.length })}
        {failed > 0 && <span className="text-red-400 ml-2">{failed} failed</span>}
      </p>

      <p className="text-xs text-muted-foreground/50 mb-4">{t('mission.selectStepToView')}</p>

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

      <div className="space-y-2">
        {mission.steps.map(step => (
          <button
            key={step.index}
            onClick={() => onSelectStep(step.index)}
            className="w-full text-left px-3 py-2 rounded-md border border-border/50 hover:bg-accent transition-colors"
          >
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground/50">
                {step.status === 'completed' ? '✓' : step.status === 'failed' ? '✗' : '○'}
              </span>
              <span className="text-sm truncate">{step.title}</span>
            </div>
            {step.output_summary && (
              <p className="text-xs text-muted-foreground/60 mt-1 line-clamp-2">{step.output_summary}</p>
            )}
          </button>
        ))}
      </div>

      {mission.error_message && (
        <p className="text-xs text-red-400/80 mt-4">{mission.error_message}</p>
      )}
    </div>
  );
}
