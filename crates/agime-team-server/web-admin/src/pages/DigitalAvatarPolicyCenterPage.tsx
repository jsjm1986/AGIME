import { useCallback, useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowLeft, Loader2, RefreshCw, ShieldAlert } from 'lucide-react';
import { AppShell } from '../components/layout/AppShell';
import { TeamProvider } from '../contexts/TeamContext';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Skeleton } from '../components/ui/skeleton';
import { Input } from '../components/ui/input';
import { apiClient } from '../api/client';
import type { AvatarGovernanceTeamSettings, TeamWithStats } from '../api/types';
import { avatarPortalApi, type AvatarInstanceProjection, type PortalSummary } from '../api/avatarPortal';
import { mergeGovernanceAutomationConfig } from '../components/team/digital-avatar/governance';
import { detectAvatarType } from '../components/team/digital-avatar/avatarType';
import { AvatarTypeBadge } from '../components/team/digital-avatar/AvatarTypeBadge';
import { useToast } from '../contexts/ToastContext';

type ApplyScope = 'all' | 'external' | 'internal';

const DEFAULT_POLICY: AvatarGovernanceTeamSettings = {
  autoProposalTriggerCount: 3,
  managerApprovalMode: 'manager_decides',
  optimizationMode: 'dual_loop',
  lowRiskAction: 'auto_execute',
  mediumRiskAction: 'manager_review',
  highRiskAction: 'human_review',
  autoCreateCapabilityRequests: true,
  autoCreateOptimizationTickets: true,
  requireHumanForPublish: true,
};

function actionLabel(
  value: AvatarGovernanceTeamSettings['lowRiskAction'],
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (value) {
    case 'auto_execute':
      return t('digitalAvatar.policy.autoExecute', { defaultValue: '自动执行' });
    case 'manager_review':
      return t('digitalAvatar.policy.managerReview', { defaultValue: '管理 Agent 决策' });
    default:
      return t('digitalAvatar.policy.humanReview', { defaultValue: '人工审批' });
  }
}

export default function DigitalAvatarPolicyCenterPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { teamId } = useParams<{ teamId: string }>();
  const { addToast } = useToast();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try {
      return window.localStorage.getItem('sidebar-collapsed') === 'true';
    } catch {
      return false;
    }
  });
  const [policy, setPolicy] = useState<AvatarGovernanceTeamSettings>(DEFAULT_POLICY);
  const [avatars, setAvatars] = useState<PortalSummary[]>([]);
  const [projections, setProjections] = useState<Map<string, AvatarInstanceProjection>>(new Map());
  const [applyScope, setApplyScope] = useState<ApplyScope>('all');
  const [savingDefaults, setSavingDefaults] = useState(false);
  const [applying, setApplying] = useState(false);

  const canManage = Boolean(team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin');

  const handleSectionChange = useCallback((section: string) => {
    if (!teamId) return;
    navigate(`/teams/${teamId}?section=${section}`);
  }, [navigate, teamId]);

  const handleToggleSidebar = useCallback(() => {
    setSidebarCollapsed((prev) => {
      try {
        window.localStorage.setItem('sidebar-collapsed', String(!prev));
      } catch {
        // ignore storage failure
      }
      return !prev;
    });
  }, []);

  const loadData = useCallback(async () => {
    if (!teamId) return;
    try {
      setLoading(true);
      const [teamResult, settingsResult, avatarResult] = await Promise.all([
        apiClient.getTeam(teamId),
        apiClient.getTeamSettings(teamId),
        avatarPortalApi.list(teamId, 1, 200),
      ]);
      const avatarProjections = await avatarPortalApi.listInstances(teamId).catch(() => []);
      setTeam(teamResult.team);
      setPolicy(settingsResult.avatarGovernance || DEFAULT_POLICY);
      setAvatars(avatarResult.items || []);
      setProjections(new Map(avatarProjections.map((item) => [item.portalId, item])));
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [t, teamId]);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  const scopedAvatars = useMemo(() => {
    if (applyScope === 'all') return avatars;
    return avatars.filter((avatar) =>
      detectAvatarType(avatar, projections.get(avatar.id)) === applyScope);
  }, [applyScope, avatars, projections]);

  const handleSaveDefaults = useCallback(async () => {
    if (!teamId || !canManage) return;
    setSavingDefaults(true);
    try {
      const result = await apiClient.updateTeamSettings(teamId, {
        avatarGovernance: policy,
      });
      setPolicy(result.avatarGovernance || policy);
      addToast('success', t('common.saved'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSavingDefaults(false);
    }
  }, [addToast, canManage, policy, t, teamId]);

  const handleApplyToExisting = useCallback(async () => {
    if (!teamId || !canManage || scopedAvatars.length === 0) return;
    setApplying(true);
    let updated = 0;
    let failed = 0;
    try {
      for (const avatar of scopedAvatars) {
        try {
          const detail = await avatarPortalApi.get(teamId, avatar.id);
          const nextConfig: Record<string, unknown> = { ...policy };
          await avatarPortalApi.updateGovernance(teamId, avatar.id, { config: nextConfig });
          await avatarPortalApi.update(teamId, avatar.id, {
            settings: mergeGovernanceAutomationConfig(
              detail.settings as Record<string, unknown> | null | undefined,
              policy,
            ),
          });
          updated += 1;
        } catch {
          failed += 1;
        }
      }
      if (updated > 0) {
        addToast(
          'success',
          t('digitalAvatar.policy.applyResult', '已同步 {{updated}} 个分身{{failedHint}}', {
            updated,
            failedHint: failed > 0 ? `，失败 ${failed} 个` : '',
          }),
        );
      } else {
        addToast('error', t('digitalAvatar.policy.applyNone', '没有分身被成功同步，请检查权限或网络状态。'));
      }
      await loadData();
    } finally {
      setApplying(false);
    }
  }, [addToast, canManage, loadData, policy, scopedAvatars, t, teamId]);

  if (loading) {
    return (
      <AppShell className="team-font-cap">
        <div className="space-y-4">
          <Skeleton className="h-12 w-72" />
          <Skeleton className="h-36 w-full" />
          <Skeleton className="h-72 w-full" />
        </div>
      </AppShell>
    );
  }

  if (!team || error) {
    return (
      <AppShell className="team-font-cap">
        <div className="flex flex-col items-center justify-center gap-4 py-16 text-center">
          <p className="text-[hsl(var(--destructive))]">{error || t('teams.notFound')}</p>
          <Link to={teamId ? `/teams/${teamId}?section=digital-avatar` : '/teams'}>
            <Button variant="outline">{t('teams.backToList')}</Button>
          </Link>
        </div>
      </AppShell>
    );
  }

  return (
    <TeamProvider
      value={{
        team,
        canManage,
        activeSection: 'digital-avatar',
        onSectionChange: handleSectionChange,
        onInviteClick: () => undefined,
        sidebarCollapsed,
        onToggleSidebar: handleToggleSidebar,
      }}
    >
      <AppShell className="team-font-cap">
        <div className="space-y-6">
          <div className="flex items-center justify-between gap-3">
            <Button variant="ghost" size="sm" className="px-2" onClick={() => navigate(`/teams/${teamId}?section=digital-avatar`)}>
              <ArrowLeft className="mr-1.5 h-4 w-4" />
              {t('digitalAvatar.policy.backToWorkspace', '返回数字分身工作台')}
            </Button>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" onClick={() => navigate(`/teams/${teamId}/digital-avatars/overview`)}>
                {t('digitalAvatar.actions.overview', '治理总览')}
              </Button>
              <Button variant="outline" size="sm" onClick={() => navigate(`/teams/${teamId}/digital-avatars/audit`)}>
                <Activity className="mr-1.5 h-4 w-4" />
                {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
              </Button>
              <Button variant="outline" size="sm" onClick={() => void loadData()}>
                <RefreshCw className="mr-1.5 h-4 w-4" />
                {t('common.refresh', '刷新')}
              </Button>
            </div>
          </div>

          <Card className="border-border/70">
            <CardHeader className="pb-3">
              <div className="flex flex-col gap-3 xl:flex-row xl:items-start xl:justify-between">
                <div className="space-y-2">
                  <CardTitle className="text-xl flex items-center gap-2">
                    <ShieldAlert className="h-5 w-5" />
                    {t('digitalAvatar.policy.title', '风险策略中心')}
                  </CardTitle>
                  <p className="text-sm text-muted-foreground">
                    {t('digitalAvatar.policy.description', '统一定义数字分身的默认治理策略，并按需批量同步到现有分身。')}
                  </p>
                </div>
                <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground xl:w-[min(34vw,360px)] xl:max-w-[360px]">
                  <div><div>{t('digitalAvatar.policy.newAvatarScope', '新建分身')}</div><div className="mt-1 font-medium text-foreground">{t('digitalAvatar.policy.newAvatarScopeHint', '自动继承团队默认策略')}</div></div>
                  <div><div>{t('digitalAvatar.policy.applyScopeLabel', '现有分身')}</div><div className="mt-1 font-medium text-foreground">{scopedAvatars.length}</div></div>
                  <div><div>{t('digitalAvatar.policy.lowRiskLabel', '低风险')}</div><div className="mt-1 font-medium text-foreground">{actionLabel(policy.lowRiskAction, t)}</div></div>
                  <div><div>{t('digitalAvatar.policy.highRiskLabel', '高风险')}</div><div className="mt-1 font-medium text-foreground">{actionLabel(policy.highRiskAction, t)}</div></div>
                </div>
              </div>
            </CardHeader>
          </Card>

          <div className="grid gap-6 xl:grid-cols-[minmax(0,1.1fr)_minmax(320px,0.9fr)]">
            <div className="space-y-6">
              <Card className="border-border/70">
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">{t('digitalAvatar.policy.riskMatrixTitle', '风险分层决策')}</CardTitle>
                </CardHeader>
                <CardContent className="grid gap-4 md:grid-cols-2">
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.lowRiskLabel', '低风险')}</label>
                    <select className="h-9 w-full rounded-md border bg-background px-2.5 text-sm" value={policy.lowRiskAction} onChange={(event) => setPolicy((current) => ({ ...current, lowRiskAction: event.target.value as AvatarGovernanceTeamSettings['lowRiskAction'] }))}>
                      <option value="auto_execute">{t('digitalAvatar.policy.autoExecute', '自动执行')}</option>
                      <option value="manager_review">{t('digitalAvatar.policy.managerReview', '管理 Agent 决策')}</option>
                      <option value="human_review">{t('digitalAvatar.policy.humanReview', '人工审批')}</option>
                    </select>
                  </div>
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.mediumRiskLabel', '中风险')}</label>
                    <select className="h-9 w-full rounded-md border bg-background px-2.5 text-sm" value={policy.mediumRiskAction} onChange={(event) => setPolicy((current) => ({ ...current, mediumRiskAction: event.target.value as AvatarGovernanceTeamSettings['mediumRiskAction'] }))}>
                      <option value="auto_execute">{t('digitalAvatar.policy.autoExecute', '自动执行')}</option>
                      <option value="manager_review">{t('digitalAvatar.policy.managerReview', '管理 Agent 决策')}</option>
                      <option value="human_review">{t('digitalAvatar.policy.humanReview', '人工审批')}</option>
                    </select>
                  </div>
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.highRiskLabel', '高风险')}</label>
                    <select className="h-9 w-full rounded-md border bg-background px-2.5 text-sm" value={policy.highRiskAction} onChange={(event) => setPolicy((current) => ({ ...current, highRiskAction: event.target.value as AvatarGovernanceTeamSettings['highRiskAction'] }))}>
                      <option value="manager_review">{t('digitalAvatar.policy.managerReview', '管理 Agent 决策')}</option>
                      <option value="human_review">{t('digitalAvatar.policy.humanReview', '人工审批')}</option>
                    </select>
                  </div>
                  <label className="flex items-center gap-3 rounded-lg border border-border/60 bg-muted/10 px-3 py-3 text-sm">
                    <input
                      type="checkbox"
                      className="h-4 w-4"
                      checked={policy.requireHumanForPublish}
                      onChange={(event) => setPolicy((current) => ({ ...current, requireHumanForPublish: event.target.checked }))}
                    />
                    <span className="space-y-1">
                      <span className="block font-medium text-foreground">
                        {t('digitalAvatar.policy.requireHumanForPublish', '发布前必须人工确认')}
                      </span>
                      <span className="block text-xs text-muted-foreground">
                        {t('digitalAvatar.policy.requireHumanForPublishHint', '对外发布默认走人工关卡，避免自动把高风险配置直接暴露给访客。')}
                      </span>
                    </span>
                  </label>
                </CardContent>
              </Card>

              <Card className="border-border/70">
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">{t('digitalAvatar.policy.automationTitle', '自动治理策略')}</CardTitle>
                </CardHeader>
                <CardContent className="grid gap-4 md:grid-cols-2">
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.autoProposalThreshold', '自动提案阈值')}</label>
                    <Input
                      type="number"
                      min={1}
                      max={10}
                      value={policy.autoProposalTriggerCount}
                      onChange={(event) => setPolicy((current) => ({
                        ...current,
                        autoProposalTriggerCount: Math.min(10, Math.max(1, Number.parseInt(event.target.value || '3', 10) || 3)),
                      }))}
                    />
                    <p className="text-xs text-muted-foreground">
                      {t('digitalAvatar.policy.autoProposalThresholdHint', '3 更激进，5 平衡，7 更保守。')}
                    </p>
                  </div>
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.managerApprovalMode', '管理者决策模式')}</label>
                    <select className="h-9 w-full rounded-md border bg-background px-2.5 text-sm" value={policy.managerApprovalMode} onChange={(event) => setPolicy((current) => ({ ...current, managerApprovalMode: event.target.value as AvatarGovernanceTeamSettings['managerApprovalMode'] }))}>
                      <option value="manager_decides">{t('digitalAvatar.policy.managerDecides', '管理 Agent 先决策')}</option>
                      <option value="human_gate">{t('digitalAvatar.policy.humanGate', '先进入人工关卡')}</option>
                    </select>
                  </div>
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.optimizationMode', '优化模式')}</label>
                    <select className="h-9 w-full rounded-md border bg-background px-2.5 text-sm" value={policy.optimizationMode} onChange={(event) => setPolicy((current) => ({ ...current, optimizationMode: event.target.value as AvatarGovernanceTeamSettings['optimizationMode'] }))}>
                      <option value="dual_loop">{t('digitalAvatar.policy.dualLoop', '分身自检 + 管理 Agent 双环')}</option>
                      <option value="manager_only">{t('digitalAvatar.policy.managerOnly', '仅管理 Agent 驱动')}</option>
                    </select>
                  </div>
                  <div className="space-y-3">
                    <label className="flex items-center gap-3 rounded-lg border border-border/60 bg-muted/10 px-3 py-3 text-sm">
                      <input
                        type="checkbox"
                        className="h-4 w-4"
                        checked={policy.autoCreateCapabilityRequests}
                        onChange={(event) => setPolicy((current) => ({ ...current, autoCreateCapabilityRequests: event.target.checked }))}
                      />
                      <span>
                        <span className="block font-medium text-foreground">{t('digitalAvatar.policy.autoCreateCapabilityRequests', '自动生成能力缺口请求')}</span>
                        <span className="block text-xs text-muted-foreground">{t('digitalAvatar.policy.autoCreateCapabilityRequestsHint', '当分身权限不足时，先生成标准化能力缺口请求供管理 Agent 或人工审批。')}</span>
                      </span>
                    </label>
                    <label className="flex items-center gap-3 rounded-lg border border-border/60 bg-muted/10 px-3 py-3 text-sm">
                      <input
                        type="checkbox"
                        className="h-4 w-4"
                        checked={policy.autoCreateOptimizationTickets}
                        onChange={(event) => setPolicy((current) => ({ ...current, autoCreateOptimizationTickets: event.target.checked }))}
                      />
                      <span>
                        <span className="block font-medium text-foreground">{t('digitalAvatar.policy.autoCreateOptimizationTickets', '自动生成优化工单')}</span>
                        <span className="block text-xs text-muted-foreground">{t('digitalAvatar.policy.autoCreateOptimizationTicketsHint', '当运行日志累计达到阈值时，自动沉淀为优化工单。')}</span>
                      </span>
                    </label>
                  </div>
                </CardContent>
              </Card>
            </div>

            <div className="space-y-6">
              <Card className="border-border/70">
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">{t('digitalAvatar.policy.scopeTitle', '应用范围')}</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="grid gap-3 md:grid-cols-3 xl:grid-cols-1">
                    <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4">
                      <div className="text-xs text-muted-foreground">{t('digitalAvatar.overview.totalAvatars', '全部分身')}</div>
                      <div className="mt-2 text-2xl font-semibold text-foreground">{avatars.length}</div>
                    </div>
                    <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4">
                      <div className="text-xs text-muted-foreground"><AvatarTypeBadge type="external" /></div>
                      <div className="mt-2 text-2xl font-semibold text-foreground">
                        {avatars.filter((avatar) =>
                          detectAvatarType(avatar, projections.get(avatar.id)) === 'external').length}
                      </div>
                    </div>
                    <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4">
                      <div className="text-xs text-muted-foreground"><AvatarTypeBadge type="internal" /></div>
                      <div className="mt-2 text-2xl font-semibold text-foreground">
                        {avatars.filter((avatar) =>
                          detectAvatarType(avatar, projections.get(avatar.id)) === 'internal').length}
                      </div>
                    </div>
                  </div>
                  <div className="space-y-2">
                    <label className="text-xs font-medium">{t('digitalAvatar.policy.applyScopeLabel', '同步范围')}</label>
                    <select className="h-9 w-full rounded-md border bg-background px-2.5 text-sm" value={applyScope} onChange={(event) => setApplyScope(event.target.value as ApplyScope)}>
                      <option value="all">{t('digitalAvatar.filters.all')}</option>
                      <option value="external">{t('digitalAvatar.filters.external')}</option>
                      <option value="internal">{t('digitalAvatar.filters.internal')}</option>
                    </select>
                    <p className="text-xs text-muted-foreground">
                      {t('digitalAvatar.policy.applyScopeHint', '选中的范围会把当前默认策略同步到现有数字分身的治理配置。')}
                    </p>
                  </div>
                  <div className="rounded-lg border border-border/60 bg-muted/10 px-3 py-3 text-sm text-muted-foreground">
                    {t('digitalAvatar.policy.applyPreview', '当前将影响 {{count}} 个分身。新建分身会自动继承这些默认值，现有分身需要手动同步。', {
                      count: scopedAvatars.length,
                    })}
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button size="sm" onClick={() => void handleSaveDefaults()} disabled={!canManage || savingDefaults}>
                      {savingDefaults ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : null}
                      {t('digitalAvatar.policy.saveDefaults', '保存为团队默认')}
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => void handleApplyToExisting()} disabled={!canManage || applying || scopedAvatars.length === 0}>
                      {applying ? <Loader2 className="mr-1.5 h-4 w-4 animate-spin" /> : null}
                      {t('digitalAvatar.policy.applyToExisting', '同步到现有分身')}
                    </Button>
                  </div>
                </CardContent>
              </Card>

              <Card className="border-border/70">
                <CardHeader className="pb-2">
                  <CardTitle className="text-base">{t('digitalAvatar.policy.currentSummary', '当前策略摘要')}</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-sm text-muted-foreground">
                  <p>{t('digitalAvatar.policy.summaryLine1', '低风险：{{value}}；中风险：{{medium}}；高风险：{{high}}。', {
                    value: actionLabel(policy.lowRiskAction, t),
                    medium: actionLabel(policy.mediumRiskAction, t),
                    high: actionLabel(policy.highRiskAction, t),
                  })}</p>
                  <p>{t('digitalAvatar.policy.summaryLine2', '管理者决策模式：{{value}}；优化模式：{{mode}}。', {
                    value: policy.managerApprovalMode === 'manager_decides'
                      ? t('digitalAvatar.policy.managerDecides', '管理 Agent 先决策')
                      : t('digitalAvatar.policy.humanGate', '先进入人工关卡'),
                    mode: policy.optimizationMode === 'dual_loop'
                      ? t('digitalAvatar.policy.dualLoop', '分身自检 + 管理 Agent 双环')
                      : t('digitalAvatar.policy.managerOnly', '仅管理 Agent 驱动'),
                  })}</p>
                  <p>{t('digitalAvatar.policy.summaryLine3', '自动提案阈值：{{count}}；能力缺口自动生成：{{req}}；优化工单自动生成：{{ticket}}。', {
                    count: policy.autoProposalTriggerCount,
                    req: policy.autoCreateCapabilityRequests ? t('common.enabled', '开启') : t('common.disabled', '关闭'),
                    ticket: policy.autoCreateOptimizationTickets ? t('common.enabled', '开启') : t('common.disabled', '关闭'),
                  })}</p>
                </CardContent>
              </Card>
            </div>
          </div>
        </div>
      </AppShell>
    </TeamProvider>
  );
}
