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
            {t('agent.execution.title', '执行策略')}
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="space-y-2 rounded-md border border-border/70 p-3">
            <Label htmlFor="approval-mode">
              {t('agent.execution.approvalMode', '审批模式')}
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
                  'Leader Owned（有 leader 时由协调链路决策）'
                )}
              </option>
              <option value="headless_fallback">
                {t(
                  'agent.execution.approvalModeHeadless',
                  'Headless Fallback（直接按 policy fallback）'
                )}
              </option>
            </select>
            <div className="text-xs text-muted-foreground">
              {policy.approvalMode === 'leader_owned'
                ? t(
                    'agent.execution.approvalModeLeaderOwnedHint',
                    'worker 的权限请求优先进入 leader/coordinator 处理链；只有没有 leader 时才允许 fallback。'
                  )
                : t(
                    'agent.execution.approvalModeHeadlessHint',
                    '适合无人值守场景，worker 权限请求可直接回退到 policy 自动判定。'
                  )}
            </div>
          </div>
          {renderToggle(
            'allowPlan',
            t('agent.execution.allowPlan', '允许 Plan 模式'),
            t(
              'agent.execution.allowPlanHint',
              '允许 Agent 在复杂任务前先进入规划步骤，再决定执行路径。'
            )
          )}
          {renderToggle(
            'allowSubagent',
            t('agent.execution.allowSubagent', '允许 Subagent'),
            t(
              'agent.execution.allowSubagentHint',
              '允许 Agent 把明确的子任务委托给有边界的辅助 worker。'
            )
          )}
          {renderToggle(
            'allowSwarm',
            t('agent.execution.allowSwarm', '允许 Swarm'),
            t(
              'agent.execution.allowSwarmHint',
              '允许 Agent 使用多 worker 并行协作。关闭后将只允许单 worker 或本地执行。'
            )
          )}
          {renderToggle(
            'allowWorkerMessaging',
            t('agent.execution.allowWorkerMessaging', '允许 Worker 互发消息'),
            t(
              'agent.execution.allowWorkerMessagingHint',
              '允许 swarm worker 在同一 run 内直接向其他 worker 或 leader 发送有边界的协作消息。'
            )
          )}
          {renderToggle(
            'allowAutoSwarm',
            t('agent.execution.allowAutoSwarm', '允许自动升级到 Swarm'),
            t(
              'agent.execution.allowAutoSwarmHint',
              '允许 planner/runtime 在任务复杂时自动把单 worker 升级成多 worker。'
            )
          )}
          {renderToggle(
            'allowValidationWorker',
            t('agent.execution.allowValidation', '允许 Validation Worker'),
            t(
              'agent.execution.allowValidationHint',
              '允许 runtime 在收尾前拉起验证 worker 做结构化验收。'
            )
          )}
          {renderToggle(
            'requireFinalReport',
            t('agent.execution.requireFinalReport', '要求最终报告'),
            t(
              'agent.execution.requireFinalReportHint',
              '开启后执行类 surface 必须完成结构化 final report 才能 completed。'
            )
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="py-3">
          <CardTitle className="text-sm">
            {t('agent.execution.budgets', '执行预算')}
          </CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 sm:grid-cols-3">
          <div className="space-y-2">
            <Label htmlFor="max-subagent-depth">
              {t('agent.execution.maxSubagentDepth', '最大委托深度')}
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
              {t('agent.execution.parallelismBudget', '并行预算')}
            </Label>
            <Input
              id="parallelism-budget"
              type="number"
              min="1"
              placeholder={t('agent.execution.unlimited', '不限')}
              value={policy.parallelismBudget ?? ''}
              onChange={(e) => setNumber('parallelismBudget', e.target.value)}
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="swarm-budget">
              {t('agent.execution.swarmBudget', 'Swarm 预算')}
            </Label>
            <Input
              id="swarm-budget"
              type="number"
              min="1"
              placeholder={t('agent.execution.unlimited', '不限')}
              value={policy.swarmBudget ?? ''}
              onChange={(e) => setNumber('swarmBudget', e.target.value)}
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
