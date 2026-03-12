import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../ui/dialog';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { Textarea } from '../../ui/textarea';
import { avatarPortalApi, type PortalDetail, type PortalDocumentAccessMode } from '../../../api/avatarPortal';
import { agentApi, type TeamAgent } from '../../../api/agent';
import { apiClient } from '../../../api/client';
import type { AvatarGovernanceTeamSettings } from '../../../api/types';
import {
  buildAvatarPublicNarrativePayload,
  splitNarrativeUseCases,
} from '../../../lib/avatarPublicNarrative';
import { splitGeneralAndDedicatedAgents } from '../agentIsolation';

type AvatarType = 'external_service' | 'internal_worker';
type AvatarRunMode = 'on_demand' | 'scheduled' | 'event_driven';

interface CreateAvatarDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  managerAgentId: string | null;
  managerAgentName?: string | null;
  onCreated: (avatar: PortalDetail) => void;
}

const DEFAULT_AVATAR_POLICY: AvatarGovernanceTeamSettings = {
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

function policyActionLabel(
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

function slugify(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, '')
    .trim()
    .replace(/[\s]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '');
}

function buildDedicatedServiceAgentName(avatarName: string): string {
  const suffix = '分身服务 Agent';
  const raw = `${avatarName} - ${suffix}`.trim();
  if (raw.length <= 100) return raw;
  return raw.slice(0, 100).trim();
}

export function CreateAvatarDialog({
  open,
  onOpenChange,
  teamId,
  managerAgentId,
  managerAgentName,
  onCreated,
}: CreateAvatarDialogProps) {
  const { t } = useTranslation();
  const [allAgents, setAllAgents] = useState<TeamAgent[]>([]);
  const [templateAgents, setTemplateAgents] = useState<TeamAgent[]>([]);
  const [loadingAgents, setLoadingAgents] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');
  const [teamAvatarPolicy, setTeamAvatarPolicy] = useState<AvatarGovernanceTeamSettings>(DEFAULT_AVATAR_POLICY);

  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [slugManual, setSlugManual] = useState(false);
  const [description, setDescription] = useState('');
  const [heroIntro, setHeroIntro] = useState('');
  const [heroUseCasesText, setHeroUseCasesText] = useState('');
  const [heroWorkingStyle, setHeroWorkingStyle] = useState('');
  const [heroCtaHint, setHeroCtaHint] = useState('');
  const [avatarType, setAvatarType] = useState<AvatarType>('external_service');
  const [runMode, setRunMode] = useState<AvatarRunMode>('on_demand');
  const [documentAccessMode, setDocumentAccessMode] = useState<PortalDocumentAccessMode>('read_only');
  const [useManagerTemplateForService, setUseManagerTemplateForService] = useState(true);
  const [serviceTemplateAgentId, setServiceTemplateAgentId] = useState('');

  const managerAgent = useMemo(
    () => allAgents.find(agent => agent.id === managerAgentId) || null,
    [allAgents, managerAgentId]
  );
  const managerTemplateAllowed = Boolean(managerAgentId);
  const effectiveManagerName = managerAgent?.name || managerAgentName || t('digitalAvatar.labels.unset');

  useEffect(() => {
    if (!open) return;
    let active = true;
    setLoadingAgents(true);
    setError('');
    Promise.all([
      agentApi.listAgents(teamId, 1, 200),
      avatarPortalApi.list(teamId, 1, 200),
      apiClient.getTeamSettings(teamId).catch(() => null),
    ])
      .then(([agentRes, avatarRes, teamSettings]) => {
        if (!active) return;
        const agents = agentRes.items || [];
        const avatars = avatarRes.items || [];
        const { generalAgents } = splitGeneralAndDedicatedAgents(agents, avatars);
        setAllAgents(agents);
        setTemplateAgents(generalAgents);
        setTeamAvatarPolicy(teamSettings?.avatarGovernance || DEFAULT_AVATAR_POLICY);
        setServiceTemplateAgentId((current) => current || generalAgents[0]?.id || '');
      })
      .catch((err) => {
        if (!active) return;
        setError(err instanceof Error ? err.message : t('common.error'));
      })
      .finally(() => {
        if (!active) return;
        setLoadingAgents(false);
      });
    return () => {
      active = false;
    };
  }, [open, teamId, t, managerAgentId]);

  useEffect(() => {
    if (slugManual) return;
    setSlug(slugify(name));
  }, [name, slugManual]);

  useEffect(() => {
    if (useManagerTemplateForService && managerAgentId) {
      setServiceTemplateAgentId(managerAgentId);
    }
  }, [managerAgentId, useManagerTemplateForService]);

  useEffect(() => {
    if (useManagerTemplateForService) return;
    if (!serviceTemplateAgentId) return;
    if (templateAgents.some(agent => agent.id === serviceTemplateAgentId)) return;
    setServiceTemplateAgentId(templateAgents[0]?.id || '');
  }, [serviceTemplateAgentId, templateAgents, useManagerTemplateForService]);

  const resetForm = () => {
    setName('');
    setSlug('');
    setSlugManual(false);
    setDescription('');
    setHeroIntro('');
    setHeroUseCasesText('');
    setHeroWorkingStyle('');
    setHeroCtaHint('');
    setAvatarType('external_service');
    setRunMode('on_demand');
    setDocumentAccessMode('read_only');
    setUseManagerTemplateForService(true);
    setServiceTemplateAgentId(templateAgents[0]?.id || '');
    setError('');
  };

  const handleCreate = async () => {
    if (!name.trim()) return;
    if (!managerAgentId) {
      setError(t('digitalAvatar.createDialog.managerRequired'));
      return;
    }
    if (!useManagerTemplateForService && !serviceTemplateAgentId) {
      setError(
        t('digitalAvatar.createDialog.serviceTemplateRequired', {
          defaultValue: '请选择一个通用 Agent 作为分身模板',
        }),
      );
      return;
    }

    const serviceTemplateId = useManagerTemplateForService
      ? managerAgentId
      : serviceTemplateAgentId;

    setSubmitting(true);
    setError('');
    let created = false;
    let serviceDedicated: TeamAgent | null = null;
    try {
      const normalizedSlug = slug.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-');
      const baseName = name.trim();
      const avatarPublicNarrative = buildAvatarPublicNarrativePayload({
        heroIntro,
        heroUseCases: splitNarrativeUseCases(heroUseCasesText),
        heroWorkingStyle,
        heroCtaHint,
      });

      serviceDedicated = await agentApi.provisionFromTemplate(serviceTemplateId, {
        name: buildDedicatedServiceAgentName(baseName),
        agent_domain: 'digital_avatar',
        agent_role: 'service',
        owner_manager_agent_id: managerAgentId,
        template_source_agent_id: serviceTemplateId,
      });

      const req = {
        name: baseName,
        slug: normalizedSlug || undefined,
        description: description.trim() || undefined,
        outputForm: 'agent_only' as const,
        agentEnabled: true,
        codingAgentId: managerAgentId,
        serviceAgentId: serviceDedicated.id,
        documentAccessMode,
        tags: [
          'digital-avatar',
          avatarType === 'external_service' ? 'avatar:external' : 'avatar:internal',
          `manager:${managerAgentId}`,
        ],
        settings: {
          avatarType,
          runMode,
          managerApprovalMode: teamAvatarPolicy.managerApprovalMode,
          optimizationMode: teamAvatarPolicy.optimizationMode,
          managerAgentId,
          managerGroupId: managerAgentId,
          serviceTemplateAgentId: serviceTemplateId,
          serviceRuntimeAgentId: serviceDedicated.id,
          digitalAvatarGovernanceConfig: {
            autoProposalTriggerCount: teamAvatarPolicy.autoProposalTriggerCount,
            managerApprovalMode: teamAvatarPolicy.managerApprovalMode,
            optimizationMode: teamAvatarPolicy.optimizationMode,
            lowRiskAction: teamAvatarPolicy.lowRiskAction,
            mediumRiskAction: teamAvatarPolicy.mediumRiskAction,
            highRiskAction: teamAvatarPolicy.highRiskAction,
            autoCreateCapabilityRequests: teamAvatarPolicy.autoCreateCapabilityRequests,
            autoCreateOptimizationTickets: teamAvatarPolicy.autoCreateOptimizationTickets,
            requireHumanForPublish: teamAvatarPolicy.requireHumanForPublish,
          },
          ...(avatarPublicNarrative ? { avatarPublicNarrative } : {}),
        },
      };
      const portal = await avatarPortalApi.create(teamId, req);
      created = true;
      onCreated(portal);
      onOpenChange(false);
      resetForm();
    } catch (err) {
      if (!created && serviceDedicated?.id) {
        await agentApi.deleteAgent(serviceDedicated.id).catch(() => undefined);
      }
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[720px]">
        <DialogHeader>
          <DialogTitle>{t('digitalAvatar.createDialog.title')}</DialogTitle>
          <DialogDescription>{t('digitalAvatar.createDialog.description')}</DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-1">
          <div className="rounded-md border bg-muted/20 p-2.5">
            <p className="text-caption text-muted-foreground">{t('digitalAvatar.labels.managerAgent')}</p>
            <p className="mt-0.5 text-xs font-medium text-foreground">{effectiveManagerName}</p>
          </div>

          <div className="rounded-md border border-primary/15 bg-primary/5 p-2.5 text-xs text-muted-foreground">
            <p className="font-medium text-foreground">
              {t('digitalAvatar.createDialog.policyInheritanceTitle', '将继承团队默认治理策略')}
            </p>
            <p className="mt-1 leading-5">
              {t('digitalAvatar.createDialog.policyInheritanceBody', '当前默认：阈值 {{count}}；低风险={{low}}；中风险={{medium}}；高风险={{high}}。创建后可在“风险策略中心”批量调整，也可在单个分身工作台里按需覆盖。', {
                count: teamAvatarPolicy.autoProposalTriggerCount,
                low: policyActionLabel(teamAvatarPolicy.lowRiskAction, t),
                medium: policyActionLabel(teamAvatarPolicy.mediumRiskAction, t),
                high: policyActionLabel(teamAvatarPolicy.highRiskAction, t),
              })}
            </p>
          </div>

          <div>
            <label className="text-xs font-medium">{t('digitalAvatar.createDialog.name')}</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('digitalAvatar.createDialog.namePlaceholder')}
            />
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.slug')}</label>
              <Input
                value={slug}
                onChange={(e) => {
                  setSlugManual(true);
                  setSlug(e.target.value);
                }}
                placeholder={t('digitalAvatar.createDialog.slugPlaceholder')}
              />
            </div>
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.type')}</label>
              <select
                className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                value={avatarType}
                onChange={(e) => setAvatarType(e.target.value as AvatarType)}
              >
                <option value="external_service">{t('digitalAvatar.types.external')}</option>
                <option value="internal_worker">{t('digitalAvatar.types.internal')}</option>
              </select>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.runMode')}</label>
              <select
                className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                value={runMode}
                onChange={(e) => setRunMode(e.target.value as AvatarRunMode)}
              >
                <option value="on_demand">{t('digitalAvatar.runModes.on_demand')}</option>
                <option value="scheduled">{t('digitalAvatar.runModes.scheduled')}</option>
                <option value="event_driven">{t('digitalAvatar.runModes.event_driven')}</option>
              </select>
            </div>
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.documentAccess')}</label>
              <select
                className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                value={documentAccessMode}
                onChange={(e) => setDocumentAccessMode(e.target.value as PortalDocumentAccessMode)}
              >
                <option value="read_only">{t('laboratory.documentAccessModeReadOnly')}</option>
                <option value="co_edit_draft">{t('laboratory.documentAccessModeCoEditDraft')}</option>
                <option value="controlled_write">{t('laboratory.documentAccessModeControlledWrite')}</option>
              </select>
            </div>
          </div>

          <div className="space-y-2 rounded-md border p-2.5">
            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={useManagerTemplateForService}
                onChange={(e) => setUseManagerTemplateForService(e.target.checked)}
                disabled={!managerTemplateAllowed}
              />
              {t('digitalAvatar.createDialog.useManagerTemplateForService', {
                defaultValue: '分身 Agent 直接沿用当前管理 Agent 模板',
              })}
            </label>
            {!managerTemplateAllowed && (
              <p className="text-caption text-muted-foreground">
                {t('digitalAvatar.createDialog.managerTemplateBlockedHint', {
                  defaultValue: '默认直接复制当前管理 Agent 的配置生成专用服务 Agent；如果你希望换一套能力，再改用通用 Agent 模板。',
                })}
              </p>
            )}

            {!useManagerTemplateForService && (
              <div>
                <label className="text-xs font-medium">{t('digitalAvatar.createDialog.serviceAgent')}</label>
                <select
                  className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                  value={serviceTemplateAgentId}
                  onChange={(e) => setServiceTemplateAgentId(e.target.value)}
                  disabled={loadingAgents}
                >
                  <option value="">{t('digitalAvatar.createDialog.noAgent')}</option>
                  {templateAgents.map(agent => (
                    <option key={agent.id} value={agent.id}>
                      {agent.name}{agent.model ? ` (${agent.model})` : ''}
                    </option>
                  ))}
                </select>
                {templateAgents.length === 0 && (
                  <p className="mt-1 text-caption text-muted-foreground">
                    {t('digitalAvatar.createDialog.noGeneralTemplateHint', {
                      defaultValue: '当前没有可用的通用 Agent 模板。请先到 Agent 管理中创建一个常规 Agent。',
                    })}
                  </p>
                )}
              </div>
            )}
            <p className="text-caption text-muted-foreground">
              {t('digitalAvatar.createDialog.serviceAgentHint', {
                defaultValue: '分身会复制所选模板 Agent 的基础配置，并生成一个独立的专用服务 Agent。',
              })}
            </p>
          </div>

          <div>
            <label className="text-xs font-medium">{t('digitalAvatar.createDialog.descriptionLabel')}</label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t('digitalAvatar.createDialog.descriptionPlaceholder')}
            />
          </div>

          <div className="space-y-3 rounded-md border border-border/70 bg-muted/10 p-3">
            <div>
              <p className="text-xs font-medium text-foreground">
                {t('digitalAvatar.createDialog.publicNarrativeTitle', '公开页顶部叙事')}
              </p>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                {t(
                  'digitalAvatar.createDialog.publicNarrativeHint',
                  '这部分会展示在对外分身页面顶部，用来解释这个分身为什么存在、适合处理什么，以及用户应该如何开始。',
                )}
              </p>
            </div>

            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.heroIntroLabel', '顶部主叙事')}</label>
              <Textarea
                rows={3}
                value={heroIntro}
                onChange={(e) => setHeroIntro(e.target.value)}
                placeholder={t(
                  'digitalAvatar.createDialog.heroIntroPlaceholder',
                  '例如：这是一个面向客户支持的服务分身，专门帮助用户基于产品资料快速定位问题、整理答案并给出下一步建议。',
                )}
              />
            </div>

            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.heroUseCasesLabel', '典型任务（每行一条）')}</label>
              <Textarea
                rows={4}
                value={heroUseCasesText}
                onChange={(e) => setHeroUseCasesText(e.target.value)}
                placeholder={t(
                  'digitalAvatar.createDialog.heroUseCasesPlaceholder',
                  '回答产品使用问题\n根据资料整理计划\n继续处理指定文档',
                )}
              />
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <div>
                <label className="text-xs font-medium">{t('digitalAvatar.createDialog.heroWorkingStyleLabel', '处理方式说明')}</label>
                <Textarea
                  rows={3}
                  value={heroWorkingStyle}
                  onChange={(e) => setHeroWorkingStyle(e.target.value)}
                  placeholder={t(
                    'digitalAvatar.createDialog.heroWorkingStylePlaceholder',
                    '例如：我会先基于当前开放资料处理；超出范围时，会继续交给管理 Agent 判断。',
                  )}
                />
              </div>
              <div>
                <label className="text-xs font-medium">{t('digitalAvatar.createDialog.heroCtaHintLabel', '开始提示')}</label>
                <Textarea
                  rows={3}
                  value={heroCtaHint}
                  onChange={(e) => setHeroCtaHint(e.target.value)}
                  placeholder={t(
                    'digitalAvatar.createDialog.heroCtaHintPlaceholder',
                    '例如：直接在对话频道描述问题；如果需要我结合资料处理，先去资料频道选中目标文档。',
                  )}
                />
              </div>
            </div>
          </div>

          {error && (
            <p className="text-xs text-[hsl(var(--destructive))]">{error}</p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button onClick={handleCreate} disabled={submitting || !name.trim() || !managerAgentId}>
            {submitting ? t('common.creating') : t('digitalAvatar.createDialog.create')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}



