import { useState, useEffect, useCallback, useRef } from 'react';
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

const statusColors: Record<MissionStatus, string> = {
  draft: 'bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-300',
  planning: 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300',
  planned: 'bg-indigo-100 text-indigo-700 dark:bg-indigo-900 dark:text-indigo-300',
  running: 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300',
  paused: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300',
  completed: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900 dark:text-emerald-300',
  failed: 'bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300',
  cancelled: 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400',
};

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
      loadMission();
    } catch (e) { console.error('Failed to start mission:', e); }
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

  const awaitingStep = mission.steps.find(s => s.status === 'awaiting_approval');
  const currentStep = mission.steps.find(s => s.index === mission.current_step);
  const completedSteps = mission.steps.filter(s => s.status === 'completed').length;
  const canStart = mission.status === 'draft' || mission.status === 'planned';
  const canPause = mission.status === 'running';
  const canCancelMission = ['planning', 'running', 'paused'].includes(mission.status);
  const canDelete = ['draft', 'cancelled', 'failed'].includes(mission.status);

  return (
    <div className="flex flex-col h-[calc(100vh-40px)] overflow-hidden">
      {/* Header */}
      <div className="p-4 border-b">
        <div className="flex items-center gap-3 mb-2">
          <Button variant="ghost" size="sm" onClick={onBack}>
            &larr; {t('mission.title')}
          </Button>
          <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${statusColors[mission.status]}`}>
            {t(`mission.${mission.status}`)}
          </span>
        </div>
        <h1 className="text-lg font-semibold line-clamp-2">{mission.goal}</h1>
        <div className="flex items-center gap-4 mt-2 text-xs text-muted-foreground">
          <span>{t('mission.tokenUsage')}: {mission.total_tokens_used.toLocaleString()}{mission.token_budget > 0 ? ` / ${mission.token_budget.toLocaleString()}` : ''}</span>
          {mission.execution_mode === 'adaptive' && mission.goal_tree ? (
            <>
              <span>{t('mission.goals')}: {mission.goal_tree.filter(g => g.status === 'completed').length}/{mission.goal_tree.length}</span>
              {mission.total_pivots > 0 && <span>{t('mission.pivots')}: {mission.total_pivots}</span>}
              {mission.total_abandoned > 0 && <span>{t('mission.abandonedCount')}: {mission.total_abandoned}</span>}
            </>
          ) : (
            <span>{t('mission.progress', { completed: completedSteps, total: mission.steps.length })}</span>
          )}
          <span className="capitalize">{mission.approval_policy}</span>
          {mission.execution_mode === 'adaptive' && (
            <span className="px-1.5 py-0.5 rounded bg-purple-100 text-purple-700 dark:bg-purple-900 dark:text-purple-300">{t('mission.adaptiveLabel')}</span>
          )}
        </div>

        {/* Plan confirmation banner */}
        {mission.status === 'planned' && mission.execution_mode === 'adaptive' && mission.goal_tree && (
          <div className="mt-2 text-sm bg-purple-50 dark:bg-purple-950/30 text-purple-700 dark:text-purple-300 rounded p-2">
            {t('mission.planReady')}
          </div>
        )}

        {/* Action buttons */}
        <div className="flex gap-2 mt-3">
          {canStart && (
            <Button size="sm" onClick={onStart}>
              {mission.status === 'planned' && mission.execution_mode === 'adaptive'
                ? t('mission.confirmExecute')
                : t('mission.start')}
            </Button>
          )}
          {canPause && (
            <Button size="sm" variant="outline" onClick={onPause}>
              {t('mission.pause')}
            </Button>
          )}
          {canCancelMission && (
            <Button size="sm" variant="outline" onClick={onCancel}>
              {t('mission.cancel')}
            </Button>
          )}
          {canDelete && (
            <Button size="sm" variant="destructive" onClick={onDelete}>
              {t('common.delete')}
            </Button>
          )}
        </div>

        {mission.error_message && (
          <div className="mt-2 text-sm text-red-600 bg-red-50 dark:bg-red-950/30 rounded p-2">
            {mission.error_message}
          </div>
        )}
      </div>

      {/* Content area */}
      <div className="flex-1 flex overflow-hidden">
        {/* Left: Steps/Goals */}
        <div className="w-80 border-r overflow-y-auto p-3">
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
                onApprove={(idx) => onApproveStep(idx)}
                onReject={(idx) => onRejectStep(idx)}
                onSkip={(idx) => onSkipStep(idx)}
              />
            </>
          )}
        </div>

        {/* Right: Tab content */}
        <div className="flex-1 flex flex-col overflow-hidden">
          <div className="flex border-b" role="tablist">
            {(['output', 'artifacts'] as const).map(tab => (
              <button
                key={tab}
                role="tab"
                aria-selected={activeTab === tab}
                onClick={() => setActiveTab(tab)}
                className={`px-4 py-2 text-sm border-b-2 transition-colors ${
                  activeTab === tab
                    ? 'border-primary text-foreground'
                    : 'border-transparent text-muted-foreground hover:text-foreground'
                }`}
              >
                {tab === 'output' ? t('mission.output') : t('mission.artifacts')}
              </button>
            ))}
          </div>

          <div className="flex-1 overflow-hidden">
            {activeTab === 'output' && currentStep && (
              <MissionStepDetail
                step={currentStep}
                missionId={missionId}
                isActive={mission.status === 'running'}
                messages={messages}
              />
            )}
            {activeTab === 'output' && !currentStep && (
              <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
                {mission.status === 'completed'
                  ? t('mission.completed')
                  : mission.status === 'draft'
                  ? t('mission.draft')
                  : t('mission.planning')}
              </div>
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