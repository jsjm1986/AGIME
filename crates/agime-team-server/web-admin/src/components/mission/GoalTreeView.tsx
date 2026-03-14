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
  running: 'text-status-info-text',
  awaiting_approval: 'text-status-warning-text',
  completed: 'text-status-success-text',
  pivoting: 'text-status-warning-text',
  abandoned: 'text-muted-foreground/65',
  failed: 'text-status-error-text',
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
              <span className="rounded px-1.5 py-0.5 text-xs border border-[hsl(var(--status-warning-text))/0.16] bg-status-warning-bg text-status-warning-text">
                checkpoint
              </span>
            )}
            {goal.status === 'pivoting' && (
              <span className="rounded px-1.5 py-0.5 text-xs border border-[hsl(var(--status-info-text))/0.16] bg-status-info-bg text-status-info-text">
                pivoting
              </span>
            )}
            {goal.status === 'abandoned' && (
              <span className="rounded-full border border-[hsl(var(--ui-line-soft))/0.75] bg-[hsl(var(--ui-surface-panel-muted))/0.86] px-1.5 py-0.5 text-xs text-muted-foreground">
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
                a.signal === 'advancing' ? 'text-status-success-text' :
                a.signal === 'stalled' ? 'text-status-warning-text' : 'text-status-error-text'
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
              className="rounded px-2 py-1 text-xs bg-primary text-primary-foreground hover:opacity-90"
            >
              {t('mission.approve')}
            </button>
          )}
          {onReject && (
            <button
              onClick={() => onReject(goal.goal_id)}
              className="rounded px-2 py-1 text-xs border border-[hsl(var(--status-error-text))/0.18] bg-[hsl(var(--status-error-bg))/0.9] text-status-error-text hover:opacity-90"
            >
              {t('mission.reject')}
            </button>
          )}
          {onPivot && (
            <button
              onClick={() => onPivot(goal.goal_id)}
              className="rounded px-2 py-1 text-xs border border-[hsl(var(--status-warning-text))/0.18] bg-status-warning-bg text-status-warning-text hover:opacity-90"
            >
              {t('mission.pivot')}
            </button>
          )}
          {onAbandon && (
            <button
              onClick={() => onAbandon(goal.goal_id)}
              className="rounded-full border border-[hsl(var(--ui-line-strong))/0.75] bg-[hsl(var(--ui-surface-panel-strong))/0.96] px-2.5 py-1 text-xs font-medium text-foreground transition-colors hover:bg-[hsl(var(--ui-surface-selected))/0.84]"
            >
              {t('mission.abandon')}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
