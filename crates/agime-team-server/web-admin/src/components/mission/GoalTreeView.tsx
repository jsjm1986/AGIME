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

function humanizeToken(value?: string | null): string {
  if (!value) return '';
  return value.replace(/_/g, ' ').replace(/\b\w/g, ch => ch.toUpperCase());
}

function goalStatusSurface(status: GoalStatus): string {
  switch (status) {
    case 'completed':
      return 'bg-status-success-bg/60 text-status-success-text';
    case 'running':
      return 'bg-[hsl(var(--status-info-text))/0.12] text-status-info-text';
    case 'awaiting_approval':
    case 'pivoting':
      return 'bg-status-warning-bg text-status-warning-text';
    case 'failed':
      return 'bg-status-error-bg/70 text-status-error-text';
    case 'abandoned':
      return 'bg-muted/40 text-muted-foreground/72';
    default:
      return 'bg-muted/28 text-muted-foreground/78';
  }
}

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
    <div className="space-y-2">
      <div className="grid grid-cols-[74px_minmax(0,1fr)_96px] gap-3 px-4 pb-1 text-[10px] uppercase tracking-[0.16em] text-muted-foreground/44">
        <span>{t('mission.goalLabel', 'Goal')}</span>
        <span>{t('mission.goalScope', 'Scope')}</span>
        <span className="text-right">{t('mission.goalState', 'State')}</span>
      </div>
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

function readableGoalStatus(status: GoalStatus, t: ReturnType<typeof useTranslation>['t']): string {
  switch (status) {
    case 'awaiting_approval':
      return t('mission.awaitingApproval');
    case 'pivoting':
      return t('mission.pivoting');
    case 'abandoned':
      return t('mission.abandoned');
    default:
      return t(`mission.${status}`, status);
  }
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
  const { t } = useTranslation();
  const indent = goal.depth * 14;
  const latestAttempt = goal.attempts[goal.attempts.length - 1];
  const latestSignal = latestAttempt ? humanizeToken(latestAttempt.signal) : t('mission.notStarted', 'Not started');
  const triesLabel = goal.attempts.length > 0
    ? t('mission.attemptsShort', {
      current: goal.attempts.length,
      total: goal.exploration_budget,
    })
    : t('mission.notStarted', 'Not started');

  return (
    <div
      className={`rounded-[22px] transition-colors ${
        isCurrent
          ? 'bg-[linear-gradient(180deg,rgba(255,255,255,0.96),rgba(250,246,238,0.92))] ring-1 ring-[hsl(var(--status-info-text))/0.16]'
          : 'bg-transparent ring-1 ring-transparent hover:bg-background/66 hover:ring-border/18'
      }`}
      style={{ marginLeft: `${indent}px` }}
    >
      <div
        className="grid cursor-pointer grid-cols-[74px_minmax(0,1fr)_96px] gap-3 px-4 py-3.5"
        onClick={onToggle}
      >
        <div className="min-w-0">
          <p className="text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground/56">
            {goal.goal_id}
          </p>
          <div className="mt-2 flex items-center gap-2">
            <span className={`inline-flex h-6 w-6 items-center justify-center rounded-full ${goalStatusSurface(goal.status)} text-[11px]`}>
              {goalStatusIcon[goal.status]}
            </span>
            {goal.is_checkpoint && (
              <span className="text-[10px] uppercase tracking-[0.14em] text-status-warning-text/88">
                {t('mission.checkpoint')}
              </span>
            )}
          </div>
        </div>
        <div className="min-w-0">
          <div className="flex items-start justify-between gap-3">
            <span className="min-w-0 flex-1 text-sm font-semibold leading-5 text-foreground">
              {goal.title}
            </span>
            <span className="pt-0.5 text-xs text-muted-foreground/58">
              {isExpanded ? '▾' : '▸'}
            </span>
          </div>
          {goal.description && (
            <p className="mt-1 line-clamp-2 break-words text-xs leading-5 text-muted-foreground/72">
              {goal.description}
            </p>
          )}
          <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] leading-5 text-muted-foreground/64">
            <span className="inline-flex items-center gap-1.5">
              <span className="uppercase tracking-[0.12em] text-muted-foreground/46">{t('mission.attempts', 'Attempts')}</span>
              <span className="font-medium text-foreground/82">{triesLabel}</span>
            </span>
            <span className="inline-flex items-center gap-1.5">
              <span className="uppercase tracking-[0.12em] text-muted-foreground/46">{t('mission.latestSignalTitle', 'Signal')}</span>
              <span className="font-medium text-foreground/82">{latestSignal}</span>
            </span>
          </div>
          {goal.pivot_reason && (goal.status === 'abandoned' || goal.status === 'pivoting') && (
            <p className="mt-2 rounded-2xl bg-muted/12 px-3 py-2 text-xs leading-5 text-muted-foreground/72">
              {goal.pivot_reason}
            </p>
          )}
        </div>
        <div className="text-right">
          <p className={`text-[11px] font-medium ${goalStatusColor[goal.status]}`}>
            {readableGoalStatus(goal.status, t)}
          </p>
          <p className="mt-2 text-[10px] uppercase tracking-[0.14em] text-muted-foreground/44">
            {goal.is_checkpoint ? t('mission.checkpoint', 'Checkpoint') : t('mission.goalLabel', 'Goal')}
          </p>
        </div>
      </div>

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
    <div className="space-y-3 border-t border-border/14 px-4 pb-4 pt-3">
      <div className="rounded-2xl bg-muted/10 px-3 py-3">
        <p className="mb-1 text-[11px] uppercase tracking-[0.14em] text-muted-foreground/54">
          {t('mission.successCriteria')}
        </p>
        <p className="line-clamp-4 break-words text-xs leading-5 text-muted-foreground/78">{goal.success_criteria}</p>
      </div>

      {goal.output_summary && (
        <div className="rounded-2xl bg-background/70 px-3 py-3 text-xs ring-1 ring-border/18">
          <p className="mb-1 text-[11px] uppercase tracking-[0.14em] text-muted-foreground/54">{t('mission.output')}</p>
          <p className="line-clamp-6 whitespace-pre-wrap break-words leading-5 text-muted-foreground/80">{goal.output_summary}</p>
        </div>
      )}

      {goal.attempts.length > 0 && (
        <div className="space-y-2">
          <span className="text-xs font-medium text-foreground/84">
            {t('mission.attempts')} ({goal.attempts.length}/{goal.exploration_budget}):
          </span>
          {goal.attempts.slice(-2).reverse().map((a) => (
            <div key={a.attempt_number} className="rounded-2xl bg-muted/10 px-3 py-3 text-xs">
              <div className="flex items-center justify-between gap-3">
                <span className="font-mono text-foreground/86">{t('mission.attemptLabel', { n: a.attempt_number })}</span>
                <span className={
                  a.signal === 'advancing' ? 'text-status-success-text' :
                  a.signal === 'stalled' ? 'text-status-warning-text' : 'text-status-error-text'
                }>
                  {humanizeToken(a.signal)}
                </span>
              </div>
              <p className="mt-2 line-clamp-4 whitespace-pre-wrap break-words leading-5 text-muted-foreground/76">{a.approach}</p>
              {a.learnings && <p className="mt-2 text-muted-foreground/70">{a.learnings}</p>}
            </div>
          ))}
        </div>
      )}

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
