import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Input } from '../ui/input';
import { Label } from '../ui/label';
import type { DelegationPolicy } from '../../api/agent';

interface Props {
  policy: DelegationPolicy;
  onChange: (next: DelegationPolicy) => void;
}

export function ExecutionPolicyPanel({ policy, onChange }: Props) {
  const { t } = useTranslation();

  const setBool = (key: keyof DelegationPolicy, value: boolean) => {
    onChange({ ...policy, [key]: value });
  };

  const setApprovalMode = (value: DelegationPolicy['approvalMode']) => {
    onChange({ ...policy, approvalMode: value });
  };

  const setNumber = (
    key: 'maxSubagentDepth' | 'parallelismBudget' | 'swarmBudget',
    raw: string
  ) => {
    const trimmed = raw.trim();
    if (!trimmed) {
      onChange({
        ...policy,
        [key]: key === 'maxSubagentDepth' ? 1 : undefined,
      });
      return;
    }
    const parsed = Number.parseInt(trimmed, 10);
    if (Number.isNaN(parsed)) return;
    onChange({ ...policy, [key]: parsed });
  };

  const renderToggle = (
    field: keyof Pick<
      DelegationPolicy,
      | 'allowPlan'
      | 'allowSubagent'
      | 'allowSwarm'
      | 'allowWorkerMessaging'
      | 'allowAutoSwarm'
      | 'allowValidationWorker'
      | 'requireFinalReport'
    >,
    label: string,
    description: string
  ) => (
    <label className="flex items-start justify-between gap-3 rounded-md border border-border/70 p-3">
      <div className="space-y-1">
        <div className="text-sm font-medium">{label}</div>
        <div className="text-xs text-muted-foreground">{description}</div>
      </div>
      <input
        type="checkbox"
        className="mt-1 h-4 w-4"
        checked={Boolean(policy[field])}
        onChange={(e) => setBool(field, e.target.checked)}
      />
    </label>
  );

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader className="py-3">
          <CardTitle className="text-sm">
            {t('agent.execution.title', 'Execution policy')}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-2 rounded-md border border-border/70 p-3">
            <Label htmlFor="approval-mode">
              {t('agent.execution.approvalMode', 'Approval mode')}
            </Label>
            <select
              id="approval-mode"
              className="h-10 w-full rounded-md border border-input bg-background px-3 text-sm"
              value={policy.approvalMode}
              onChange={(e) =>
                setApprovalMode(
                  e.target.value as DelegationPolicy['approvalMode']
                )
              }
            >
              <option value="leader_owned">
                {t(
                  'agent.execution.approvalModeLeaderOwned',
                  'Leader owned (decide through the coordinator path when a leader exists)'
                )}
              </option>
              <option value="headless_fallback">
                {t(
                  'agent.execution.approvalModeHeadless',
                  'Headless fallback (fallback directly by policy)'
                )}
              </option>
            </select>
            <div className="text-xs text-muted-foreground">
              {policy.approvalMode === 'leader_owned'
                ? t(
                    'agent.execution.approvalModeLeaderOwnedHint',
                    'Worker permission requests first go through the leader/coordinator path; fallback is only allowed when no leader exists.'
                  )
                : t(
                    'agent.execution.approvalModeHeadlessHint',
                    'Best for unattended scenarios where worker permission requests can fall back directly to policy evaluation.'
                  )}
            </div>
          </div>
          {renderToggle(
            'allowPlan',
            t('agent.execution.allowPlan', 'Allow plan mode'),
            t(
              'agent.execution.allowPlanHint',
              'Allow the agent to enter a planning step before deciding the execution path for complex tasks.'
            )
          )}
          {renderToggle(
            'allowSubagent',
            t('agent.execution.allowSubagent', 'Allow subagents'),
            t(
              'agent.execution.allowSubagentHint',
              'Allow the agent to delegate well-bounded subtasks to helper workers.'
            )
          )}
          {renderToggle(
            'allowSwarm',
            t('agent.execution.allowSwarm', 'Allow swarm'),
            t(
              'agent.execution.allowSwarmHint',
              'Allow the agent to use multiple workers in parallel. When disabled, only single-worker or local execution is allowed.'
            )
          )}
          {renderToggle(
            'allowWorkerMessaging',
            t('agent.execution.allowWorkerMessaging', 'Allow worker messaging'),
            t(
              'agent.execution.allowWorkerMessagingHint',
              'Allow swarm workers to send bounded collaboration messages directly to other workers or the leader within the same run.'
            )
          )}
          {renderToggle(
            'allowAutoSwarm',
            t('agent.execution.allowAutoSwarm', 'Allow automatic swarm upgrade'),
            t(
              'agent.execution.allowAutoSwarmHint',
              'Allow the planner/runtime to upgrade a single worker into multiple workers when a task becomes complex.'
            )
          )}
          {renderToggle(
            'allowValidationWorker',
            t('agent.execution.allowValidation', 'Allow validation workers'),
            t(
              'agent.execution.allowValidationHint',
              'Allow the runtime to start a validation worker before completion for structured acceptance checks.'
            )
          )}
          {renderToggle(
            'requireFinalReport',
            t('agent.execution.requireFinalReport', 'Require final report'),
            t(
              'agent.execution.requireFinalReportHint',
              'When enabled, execution surfaces must produce a structured final report before they can complete.'
            )
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="py-3">
          <CardTitle className="text-sm">
            {t('agent.execution.budgets', 'Execution budgets')}
          </CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 sm:grid-cols-3">
          <div className="space-y-2">
            <Label htmlFor="max-subagent-depth">
              {t('agent.execution.maxSubagentDepth', 'Maximum delegation depth')}
            </Label>
            <Input
              id="max-subagent-depth"
              type="number"
              min="1"
              value={policy.maxSubagentDepth ?? 1}
              onChange={(e) => setNumber('maxSubagentDepth', e.target.value)}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="parallelism-budget">
              {t('agent.execution.parallelismBudget', 'Parallelism budget')}
            </Label>
            <Input
              id="parallelism-budget"
              type="number"
              min="1"
              placeholder={t('agent.execution.unlimited', 'Unlimited')}
              value={policy.parallelismBudget ?? ''}
              onChange={(e) => setNumber('parallelismBudget', e.target.value)}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="swarm-budget">
              {t('agent.execution.swarmBudget', 'Swarm budget')}
            </Label>
            <Input
              id="swarm-budget"
              type="number"
              min="1"
              placeholder={t('agent.execution.unlimited', 'Unlimited')}
              value={policy.swarmBudget ?? ''}
              onChange={(e) => setNumber('swarmBudget', e.target.value)}
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
