import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { GoalNode, GoalStatus } from '../../api/mission';

interface GoalTreeViewProps {
  goals: GoalNode[];
  currentGoalId?: string;
  onApprove?: (goalId: string) => void;
  onReject?: (goalId: string) => void;
  onPivot?: (goalId: string) => void;
  onAbandon?: (goalId: string) => void;
}

const goalStatusIcon: Record<GoalStatus, string> = {
  pending: '○',
  running: '▶',
  awaiting_approval: '⏸',
  completed: '✓',
  pivoting: '↻',
  abandoned: '⊘',
  failed: '✗',
};

const goalStatusColor: Record<GoalStatus, string> = {
  pending: 'text-muted-foreground',
  running: 'text-blue-600 dark:text-blue-400',
  awaiting_approval: 'text-yellow-600 dark:text-yellow-400',
  completed: 'text-green-600 dark:text-green-400',
  pivoting: 'text-orange-600 dark:text-orange-400',
  abandoned: 'text-gray-400',
  failed: 'text-red-600 dark:text-red-400',
};

export function GoalTreeView({
  goals,
  currentGoalId,
  onApprove,
  onReject,
  onPivot,
  onAbandon,
}: GoalTreeViewProps) {
  const { t } = useTranslation();
  const [expandedGoals, setExpandedGoals] = useState<Set<string>>(new Set());

  if (goals.length === 0) {
    return (
      <div className="text-sm text-muted-foreground py-4 text-center">
        {t('mission.decomposingGoals')}
      </div>
    );
  }

  const toggleExpand = (goalId: string) => {
    setExpandedGoals(prev => {
      const next = new Set(prev);
      if (next.has(goalId)) next.delete(goalId);
      else next.add(goalId);
      return next;
    });
  };

  return (
    <div className="space-y-1">
      {goals.map((goal) => (
        <GoalNodeItem
          key={goal.goal_id}
          goal={goal}
          isCurrent={currentGoalId === goal.goal_id}
          isExpanded={expandedGoals.has(goal.goal_id)}
          onToggle={() => toggleExpand(goal.goal_id)}
          onApprove={onApprove}
          onReject={onReject}
          onPivot={onPivot}
          onAbandon={onAbandon}
        />
      ))}
    </div>
  );
}

interface GoalNodeItemProps {
  goal: GoalNode;
  isCurrent: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  onApprove?: (goalId: string) => void;
  onReject?: (goalId: string) => void;
  onPivot?: (goalId: string) => void;
  onAbandon?: (goalId: string) => void;
}

function GoalNodeItem({
  goal,
  isCurrent,
  isExpanded,
  onToggle,
  onApprove,
  onReject,
  onPivot,
  onAbandon,
}: GoalNodeItemProps) {
  const indent = goal.depth * 16;

  return (
    <div
      className={`rounded-md transition-colors ${isCurrent ? 'bg-accent' : ''}`}
      style={{ paddingLeft: `${indent}px` }}
    >
      {/* Goal header */}
      <div
        className="flex items-start gap-2 p-2 cursor-pointer"
        onClick={onToggle}
      >
        <span className={`text-base font-mono pt-0.5 ${goalStatusColor[goal.status]}`}>
          {goalStatusIcon[goal.status]}
        </span>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium truncate">
              {goal.goal_id}: {goal.title}
            </span>
            {goal.is_checkpoint && (
              <span className="text-xs px-1.5 py-0.5 rounded bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300">
                checkpoint
              </span>
            )}
            {goal.status === 'pivoting' && (
              <span className="text-xs px-1.5 py-0.5 rounded bg-orange-100 text-orange-700 dark:bg-orange-900 dark:text-orange-300">
                pivoting
              </span>
            )}
            {goal.status === 'abandoned' && (
              <span className="text-xs px-1.5 py-0.5 rounded bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400">
                abandoned
              </span>
            )}
          </div>

          {/* Pivot reason */}
          {goal.pivot_reason && (goal.status === 'abandoned' || goal.status === 'pivoting') && (
            <p className="text-xs text-muted-foreground mt-0.5 italic">
              {goal.pivot_reason}
            </p>
          )}
        </div>
        <span className="text-xs text-muted-foreground">
          {isExpanded ? '▾' : '▸'}
        </span>
      </div>

      {/* Expanded details */}
      {isExpanded && (
        <GoalDetails
          goal={goal}
          onApprove={onApprove}
          onReject={onReject}
          onPivot={onPivot}
          onAbandon={onAbandon}
        />
      )}
    </div>
  );
}

function GoalDetails({
  goal,
  onApprove,
  onReject,
  onPivot,
  onAbandon,
}: {
  goal: GoalNode;
  onApprove?: (goalId: string) => void;
  onReject?: (goalId: string) => void;
  onPivot?: (goalId: string) => void;
  onAbandon?: (goalId: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="pl-8 pr-2 pb-2 space-y-2">
      <p className="text-xs text-muted-foreground">{goal.description}</p>
      <p className="text-xs">
        <span className="font-medium">{t('mission.successCriteria')}:</span>{' '}
        {goal.success_criteria}
      </p>

      {/* Output summary */}
      {goal.output_summary && (
        <div className="text-xs bg-muted/50 rounded p-2">
          <span className="font-medium">{t('mission.output')}:</span> {goal.output_summary}
        </div>
      )}

      {/* Attempts history */}
      {goal.attempts.length > 0 && (
        <div className="space-y-1">
          <span className="text-xs font-medium">
            {t('mission.attempts')} ({goal.attempts.length}/{goal.exploration_budget}):
          </span>
          {goal.attempts.map((a) => (
            <div key={a.attempt_number} className="text-xs pl-2 border-l-2 border-muted">
              <span className="font-mono">{a.approach}</span>
              {' → '}
              <span className={
                a.signal === 'advancing' ? 'text-green-600' :
                a.signal === 'stalled' ? 'text-yellow-600' : 'text-red-600'
              }>
                {a.signal}
              </span>
              {a.learnings && (
                <p className="text-muted-foreground mt-0.5">{a.learnings}</p>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Approval actions */}
      {goal.status === 'awaiting_approval' && (
        <div className="flex gap-2 mt-2">
          {onApprove && (
            <button
              onClick={() => onApprove(goal.goal_id)}
              className="text-xs px-2 py-1 rounded bg-green-600 text-white hover:bg-green-700"
            >
              {t('mission.approve')}
            </button>
          )}
          {onReject && (
            <button
              onClick={() => onReject(goal.goal_id)}
              className="text-xs px-2 py-1 rounded bg-red-600 text-white hover:bg-red-700"
            >
              {t('mission.reject')}
            </button>
          )}
          {onPivot && (
            <button
              onClick={() => onPivot(goal.goal_id)}
              className="text-xs px-2 py-1 rounded bg-orange-600 text-white hover:bg-orange-700"
            >
              {t('mission.pivot')}
            </button>
          )}
          {onAbandon && (
            <button
              onClick={() => onAbandon(goal.goal_id)}
              className="text-xs px-2 py-1 rounded bg-gray-500 text-white hover:bg-gray-600"
            >
              {t('mission.abandon')}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
