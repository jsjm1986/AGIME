import { useState, useEffect, useCallback, useRef } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Sidebar } from '../components/layout/Sidebar';
import { MissionStepList } from '../components/mission/MissionStepList';
import { MissionStepDetail } from '../components/mission/MissionStepDetail';
import { ArtifactList } from '../components/mission/ArtifactList';
import { StepApprovalPanel } from '../components/mission/StepApprovalPanel';
import { GoalTreeView } from '../components/mission/GoalTreeView';
import {
  missionApi,
  MissionDetail,
  MissionStatus,
  GoalStatus,
} from '../api/mission';
import { ConfirmDialog } from '../components/ui/confirm-dialog';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

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

export default function MissionDetailPage() {
  const { teamId, missionId } = useParams<{ teamId: string; missionId: string }>();
  const navigate = useNavigate();
  const { t } = useTranslation();

  const [mission, setMission] = useState<MissionDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [messages, setMessages] = useState<StreamMessage[]>([]);
  const [activeTab, setActiveTab] = useState<'output' | 'artifacts'>('output');
  const [toolCallCount, setToolCallCount] = useState(0);
  const [selectedStepIndex, setSelectedStepIndex] = useState<number | null>(null);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);

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
    loadMission();
  }, [loadMission]);

  // Auto-follow current step during live execution
  useEffect(() => {
    if (mission && ['planning', 'running'].includes(mission.status)) {
      setSelectedStepIndex(null);
    }
  }, [mission?.current_step]);

  // SSE streaming
  useEffect(() => {
    if (!missionId || !mission) return;
    const isLive = ['planning', 'running'].includes(mission.status);
    if (!isLive) return;

    const es = missionApi.streamMission(missionId);
    eventSourceRef.current = es;

    const handleEvent = (type: string) => (e: MessageEvent) => {
      try {
        const data = JSON.parse(e.data);
        if (type === 'status') {
          loadMission();
          return;
        }
        if (type === 'done') {
          loadMission();
          es.close();
          return;
        }
        if (type === 'toolcall') {
          setToolCallCount(prev => prev + 1);
        }
        setMessages(prev => [...prev, {
          type,
          content: data.text || data.content || data.tool_name || JSON.stringify(data),
          timestamp: Date.now(),
        }]);
      } catch {
        // ignore parse errors
      }
    };

    // ─── AGE goal event handlers: update goal tree in-place ───
    const handleGoalStart = (e: MessageEvent) => {
      try {
        const data = JSON.parse(e.data);
        setMission(prev => {
          if (!prev?.goal_tree) return prev;
          return {
            ...prev,
            current_goal_id: data.goal_id,
            goal_tree: prev.goal_tree.map(g =>
              g.goal_id === data.goal_id
                ? { ...g, status: 'running' as GoalStatus }
                : g
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
              g.goal_id === data.goal_id
                ? { ...g, status: 'completed' as GoalStatus }
                : g
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
    // AGE goal events — real-time in-place updates
    es.addEventListener('goal_start', handleGoalStart);
    es.addEventListener('goal_complete', handleGoalComplete);
    es.addEventListener('pivot', handlePivot);
    es.addEventListener('goal_abandoned', handleGoalAbandoned);
    es.onerror = () => {
      es.close();
      loadMission();
    };

    return () => {
      es.close();
      eventSourceRef.current = null;
    };
  }, [missionId, mission?.status]);

  // Action handlers
  const handleStart = async () => {
    if (!missionId) return;
    try {
      await missionApi.startMission(missionId);
      setMessages([]);
      setToolCallCount(0);
      loadMission();
    } catch (e) {
      console.error('Failed to start mission:', e);
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

  const handleDelete = async () => {
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
  const displayStep = selectedStepIndex !== null
    ? mission.steps.find(s => s.index === selectedStepIndex) || currentStep
    : currentStep;
  const isDisplayStepActive = displayStep != null && currentStep != null && displayStep.index === currentStep.index && mission.status === 'running';
  const completedSteps = mission.steps.filter(s => s.status === 'completed').length;
  const canStart = mission.status === 'draft' || mission.status === 'planned';
  const canPause = mission.status === 'running';
  const canCancel = ['planning', 'running', 'paused'].includes(mission.status);
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
            <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${statusColors[mission.status]}`}>
              {t(`mission.${mission.status}`)}
            </span>
          </div>
          <h1 className="text-lg font-semibold line-clamp-2">{mission.goal}</h1>
          <div className="flex items-center gap-4 mt-2 text-xs text-muted-foreground">
            {['planning', 'running'].includes(mission.status) && toolCallCount > 0 && (
              <span>{t('mission.toolCalls', { count: toolCallCount })}</span>
            )}
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
              <button onClick={handleStart} className="px-3 py-1.5 text-sm rounded-md bg-green-600 text-white hover:bg-green-700">
                {mission.status === 'planned' && mission.execution_mode === 'adaptive'
                  ? t('mission.confirmExecute')
                  : t('mission.start')}
              </button>
            )}
            {canPause && (
              <button onClick={handlePause} className="px-3 py-1.5 text-sm rounded-md bg-yellow-600 text-white hover:bg-yellow-700">
                {t('mission.pause')}
              </button>
            )}
            {canCancel && (
              <button onClick={handleCancel} className="px-3 py-1.5 text-sm rounded-md border hover:bg-accent">
                {t('mission.cancel', 'Cancel')}
              </button>
            )}
            {canDelete && (
              <button onClick={handleDelete} className="px-3 py-1.5 text-sm rounded-md text-red-600 border border-red-200 hover:bg-red-50 dark:hover:bg-red-950">
                {t('common.delete', 'Delete')}
              </button>
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
          {/* Left: Steps */}
          <div className="w-80 border-r overflow-y-auto p-3">
            {mission.execution_mode === 'adaptive' && mission.goal_tree ? (
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
              {(['output', 'artifacts'] as const).map(tab => (
                <button
                  key={tab}
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

            {/* Tab content */}
            <div className="flex-1 overflow-hidden">
              {activeTab === 'output' && displayStep && (
                <MissionStepDetail
                  step={displayStep}
                  missionId={missionId}
                  isActive={isDisplayStepActive}
                  messages={isDisplayStepActive ? messages : []}
                />
              )}
              {activeTab === 'output' && !displayStep && (
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
