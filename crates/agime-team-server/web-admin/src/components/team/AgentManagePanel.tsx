import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Skeleton } from '../ui/skeleton';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { StatusBadge } from '../ui/status-badge';
import { AgentTypeBadge, resolveAgentVisualType } from '../agent/AgentTypeBadge';
import { AgentAvatar } from '../agent/AvatarPicker';
import { CreateAgentDialog } from '../agent/CreateAgentDialog';
import { EditAgentDialog } from '../agent/EditAgentDialog';
import { DeleteAgentDialog } from '../agent/DeleteAgentDialog';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import {
  agentApi,
  TeamAgent,
  BUILTIN_EXTENSIONS,
} from '../../api/agent';
import { apiClient } from '../../api/client';
import { portalApi } from '../../api/portal';
import {
  UNGROUPED_MANAGER_KEY,
  buildDedicatedAvatarGrouping,
  splitGeneralAndDedicatedAgents,
  type DedicatedAvatarGroup,
} from './agentIsolation';

interface AgentManagePanelProps {
  teamId: string;
  onOpenChat?: (agent: TeamAgent) => void;
  onOpenDigitalAvatar?: () => void;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function getDedicatedAgentDisplayName(agent: TeamAgent | null | undefined, fallback = 'N/A'): string {
  if (!agent?.name) return fallback;

  const visualType = resolveAgentVisualType(agent);
  const suffixes = [
    '管理Agent',
    '管理 Agent',
    '分身管理Agent',
    '分身管理 Agent',
    '服务Agent',
    '服务 Agent',
    '分身服务Agent',
    '分身服务 Agent',
    'Manager',
    'Service',
  ];

  let normalized = agent.name.trim();
  for (const suffix of suffixes) {
    normalized = normalized.replace(
      new RegExp(`(?:\\s*[-－—]\\s*${escapeRegExp(suffix)})+$`, 'giu'),
      '',
    ).trim();
  }

  if (visualType === 'avatar_manager') {
    normalized = normalized
      .replace(/(?:管理\s*Agent)(?:\s*[-－—]?\s*管理\s*Agent)+$/giu, '管理Agent')
      .replace(/(?:分身管理\s*Agent)(?:\s*[-－—]?\s*分身管理\s*Agent)+$/giu, '分身管理Agent')
      .replace(/(?:manager)(?:\s*[-－—]?\s*manager)+$/giu, 'Manager')
      .trim();
  }

  if (visualType === 'avatar_service') {
    normalized = normalized
      .replace(/(?:服务\s*Agent)(?:\s*[-－—]?\s*服务\s*Agent)+$/giu, '服务Agent')
      .replace(/(?:分身服务\s*Agent)(?:\s*[-－—]?\s*分身服务\s*Agent)+$/giu, '分身服务Agent')
      .replace(/(?:service)(?:\s*[-－—]?\s*service)+$/giu, 'Service')
      .trim();
  }

  return normalized || agent.name || fallback;
}

function formatMetricValue(value: number | string | null | undefined, fallback = '—'): string {
  if (value === null || value === undefined) return fallback;
  const text = String(value).trim();
  return text.length > 0 ? text : fallback;
}

function shortenApiEndpoint(value?: string | null): string | null {
  if (!value) return null;
  try {
    const parsed = new URL(value);
    return `${parsed.host}${parsed.pathname === '/' ? '' : parsed.pathname}`;
  } catch {
    return value;
  }
}

function getAgentStatusAccent(status: string): string {
  switch (status) {
    case 'running':
      return 'bg-[hsl(var(--status-success-text))]';
    case 'paused':
      return 'bg-[hsl(var(--status-warning-text))]';
    case 'error':
      return 'bg-[hsl(var(--status-error-text))]';
    default:
      return 'bg-[hsl(var(--ui-line-strong))]';
  }
}

function getCapacityBadge(agent: TeamAgent, t: TFunction) {
  const max = Math.max(1, agent.max_concurrent_tasks ?? 1);
  const active = Math.max(0, agent.active_execution_slots ?? 0);
  if (active <= 0) {
    return {
      status: 'neutral' as const,
      label: t('agent.capacity.idle', '空闲'),
    };
  }
  if (active >= max) {
    return {
      status: 'warning' as const,
      label: t('agent.capacity.full', '满载 {{active}}/{{max}}', { active, max }),
    };
  }
  return {
    status: 'info' as const,
    label: t('agent.capacity.running', '运行中 {{active}}/{{max}}', { active, max }),
  };
}

export function AgentManagePanel({ teamId, onOpenChat, onOpenDigitalAvatar }: AgentManagePanelProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [dedicatedGroups, setDedicatedGroups] = useState<DedicatedAvatarGroup[]>([]);
  const [ecosystemDedicatedAgents, setEcosystemDedicatedAgents] = useState<TeamAgent[]>([]);
  const [hiddenDedicatedCount, setHiddenDedicatedCount] = useState(0);
  const [showDedicatedAgents, setShowDedicatedAgents] = useState(false);
  const [dedicatedManagerFilter, setDedicatedManagerFilter] = useState('__all__');
  const [defaultGeneralAgentId, setDefaultGeneralAgentId] = useState('');
  const [aiDescribeAgentId, setAiDescribeAgentId] = useState('');
  const [documentAnalysisAgentId, setDocumentAnalysisAgentId] = useState('');
  const [documentAnalysisEnabled, setDocumentAnalysisEnabled] = useState(false);
  const [defaultAgentDetailOpen, setDefaultAgentDetailOpen] = useState(false);
  const [defaultAgentDetailTarget, setDefaultAgentDetailTarget] = useState<TeamAgent | null>(null);
  const [loading, setLoading] = useState(true);

  const [createAgentOpen, setCreateAgentOpen] = useState(false);
  const [editAgentOpen, setEditAgentOpen] = useState(false);
  const [deleteAgentOpen, setDeleteAgentOpen] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(null);

  const loadAgents = async () => {
    try {
      setLoading(true);
      const [agentResult, avatarResult, settings] = await Promise.all([
        agentApi.listAgents(teamId),
        portalApi.list(teamId, 1, 200, 'avatar'),
        apiClient.getTeamSettings(teamId),
      ]);
      const allAgents = agentResult.items || [];
      const avatarPortals = avatarResult.items || [];
      const isolation = splitGeneralAndDedicatedAgents(allAgents, avatarPortals);
      const grouping = buildDedicatedAvatarGrouping(allAgents, avatarPortals);
      setAgents(isolation.generalAgents);
      setDedicatedGroups(grouping.dedicatedGroups);
      setEcosystemDedicatedAgents(isolation.ecosystemDedicatedAgents);
      setHiddenDedicatedCount(isolation.dedicatedAgentIds.size);
      setDefaultGeneralAgentId(settings.generalAgent?.defaultAgentId || '');
      setAiDescribeAgentId(settings.aiDescribe?.agentId || '');
      setDocumentAnalysisAgentId(settings.documentAnalysis?.agentId || '');
      setDocumentAnalysisEnabled(Boolean(settings.documentAnalysis?.enabled));
    } catch (error) {
      console.error('Failed to load agents:', error);
      setAgents([]);
      setDedicatedGroups([]);
      setEcosystemDedicatedAgents([]);
      setHiddenDedicatedCount(0);
      setDefaultGeneralAgentId('');
      setAiDescribeAgentId('');
      setDocumentAnalysisAgentId('');
      setDocumentAnalysisEnabled(false);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadAgents();
  }, [teamId]);

  const getEnabledExtensionNames = (agent: TeamAgent) => {
    const enabled = agent.enabled_extensions?.filter(e => e.enabled) || [];
    return enabled.map(e => {
      const ext = BUILTIN_EXTENSIONS.find(b => b.id === e.extension);
      return ext?.name || e.extension;
    });
  };

  const getAttachedTeamExtensionNames = (agent: TeamAgent) =>
    (agent.attached_team_extensions || [])
      .filter((entry) => entry.enabled)
      .map((entry) => entry.display_name || entry.runtime_name || entry.extension_id);

  const getEnabledSkillNames = (agent: TeamAgent) =>
    (agent.assigned_skills || [])
      .filter(skill => skill.enabled)
      .map(skill => skill.name);

  const getCapacityStatusBadge = (agent: TeamAgent) => {
    const capacity = getCapacityBadge(agent, t);
    return (
      <StatusBadge status={capacity.status}>
        {capacity.label}
      </StatusBadge>
    );
  };

  const getDefaultAgentResponsibilities = (agent: TeamAgent) => {
    const items: Array<{ key: string; title: string; description: string }> = [];

    if (agent.id === defaultGeneralAgentId) {
      items.push({
        key: 'general',
        title: t('agent.manage.defaultAgentResponsibilities.general.title', '通用工作流默认 Agent'),
        description: t(
          'agent.manage.defaultAgentResponsibilities.general.description',
          'MCP 工作区等通用工作流会优先使用这个 Agent 作为默认驱动 Agent。'
        ),
      });
    }

    if (agent.id === aiDescribeAgentId) {
      items.push({
        key: 'aiDescribe',
        title: t('agent.manage.defaultAgentResponsibilities.aiDescribe.title', 'AI 洞察默认 Agent'),
        description: t(
          'agent.manage.defaultAgentResponsibilities.aiDescribe.description',
          '技能、扩展和 AI 洞察相关描述会优先使用这个 Agent 生成。'
        ),
      });
    }

    if (documentAnalysisEnabled && agent.id === documentAnalysisAgentId) {
      items.push({
        key: 'documentAnalysis',
        title: t('agent.manage.defaultAgentResponsibilities.documentAnalysis.title', '文档解读 Agent'),
        description: t(
          'agent.manage.defaultAgentResponsibilities.documentAnalysis.description',
          '上传文档后的自动解读与文档分析任务会使用这个 Agent。'
        ),
      });
    }

    return items;
  };

  const renderAgentCard = (agent: TeamAgent, roles: Array<'manager' | 'service'> = []) => {
    const enabledExtensionNames = getEnabledExtensionNames(agent);
    const attachedTeamExtensionNames = getAttachedTeamExtensionNames(agent);
    const enabledSkillNames = getEnabledSkillNames(agent);
    const enabledCustomExtensions = agent.custom_extensions?.filter(e => e.enabled) || [];
    const isDefaultGeneralAgent = roles.length === 0 && agent.id === defaultGeneralAgentId;
    const responsibilities = roles.length === 0 ? getDefaultAgentResponsibilities(agent) : [];
    const extensionChips = [
      ...enabledExtensionNames.map((name) => ({ key: `builtin-${name}`, label: name, variant: 'secondary' as const })),
      ...attachedTeamExtensionNames.map((name) => ({ key: `team-${name}`, label: name, variant: 'outline' as const })),
      ...enabledCustomExtensions.map((ext) => ({ key: `custom-${ext.name}`, label: ext.name, variant: 'outline' as const })),
    ];
    const skillChips = enabledSkillNames.map((name) => ({
      key: `skill-${name}`,
      label: name,
      className:
        'border-[hsl(var(--status-warning-text))/0.18] bg-[hsl(var(--status-warning-bg))/0.92] text-status-warning-text',
    }));
    const visualType = roles.length > 0 ? (roles[0] === 'manager' ? 'avatar_manager' : 'avatar_service') : resolveAgentVisualType(agent);
    const primaryMeta = [
      { key: 'apiFormat', label: t('agent.create.apiFormat'), value: formatMetricValue(agent.api_format) },
      {
        key: 'allowedGroups',
        label: t('agent.access.allowedGroups'),
        value: agent.allowed_groups?.length ? `${agent.allowed_groups.length}` : t('agent.access.title'),
      },
      ...(agent.api_url
        ? [{ key: 'apiUrl', label: t('agent.create.apiUrl'), value: shortenApiEndpoint(agent.api_url) ?? '—' }]
        : []),
    ];

    return (
      <Card
        key={agent.id}
        className="relative overflow-hidden border-[hsl(var(--ui-line-soft))/0.82] bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel-strong))_0%,hsl(var(--ui-surface-panel))_100%)] shadow-[0_22px_40px_-34px_hsl(var(--ui-shadow)/0.42)] transition-all duration-200 hover:border-[hsl(var(--ui-line-strong))/0.82] hover:shadow-[0_24px_46px_-34px_hsl(var(--ui-shadow)/0.5)]"
      >
        <div className={`absolute inset-x-0 top-0 h-[2px] ${getAgentStatusAccent(agent.status)}`} />
        <CardHeader className="space-y-0 p-0">
          <div className="border-b border-[hsl(var(--ui-line-soft))/0.68] px-5 py-4">
            <CardTitle className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
              <div className="min-w-0 space-y-3">
                <div className="flex items-start gap-3">
                  <div className="mt-0.5 shrink-0">
                    <div className="flex h-11 w-11 items-center justify-center rounded-[14px] border border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-muted))/0.78]">
                      <AgentAvatar avatar={agent.avatar} name={agent.name} className="h-11 w-11" iconSize="w-4.5 h-4.5" />
                    </div>
                  </div>
                  <div className="min-w-0 space-y-2">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate text-[18px] font-semibold tracking-[-0.015em] text-foreground">
                        {agent.name}
                      </span>
                      <AgentTypeBadge type={visualType} />
                      {isDefaultGeneralAgent && (
                        <button
                          type="button"
                          className="inline-flex items-center rounded-full border border-primary/24 bg-primary/8 px-2.5 py-1 text-[11px] font-medium text-primary transition-colors hover:border-primary/42 hover:bg-primary/12"
                          onClick={() => {
                            setDefaultAgentDetailTarget(agent);
                            setDefaultAgentDetailOpen(true);
                          }}
                        >
                          {t('agent.manage.defaultGeneralAgent', '默认工作流 Agent')}
                        </button>
                      )}
                      {getCapacityStatusBadge(agent)}
                    </div>

                    {agent.description ? (
                      <p className="max-w-3xl text-[13px] leading-6 text-[hsl(var(--ui-text-secondary))] line-clamp-2">
                        {agent.description}
                      </p>
                    ) : (
                      <p className="text-[12px] italic text-[hsl(var(--ui-text-tertiary))]">
                        {t('agent.noDescription')}
                      </p>
                    )}

                    {responsibilities.length > 0 && (
                      <div className="flex flex-wrap gap-2">
                        {responsibilities.map((item) => (
                          <button
                            key={item.key}
                            type="button"
                            onClick={() => {
                              setDefaultAgentDetailTarget(agent);
                              setDefaultAgentDetailOpen(true);
                            }}
                            className="inline-flex items-center rounded-full border border-[hsl(var(--status-info-text))/0.18] bg-[hsl(var(--status-info-bg))/0.88] px-2.5 py-1 text-[11px] font-medium text-[hsl(var(--status-info-text))] transition-colors hover:border-[hsl(var(--status-info-text))/0.3]"
                          >
                            {item.title}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              </div>

                <div className="flex shrink-0 items-center gap-2 self-start">
                  {onOpenChat && (
                  <button
                    type="button"
                    className="agent-manage-chat-button inline-flex min-w-[88px] items-center justify-center rounded-[10px] px-3 py-2 text-[12px] font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[hsl(var(--ring))] focus-visible:ring-offset-2"
                    onClick={() => onOpenChat(agent)}
                  >
                    {t('agent.chat.button')}
                  </button>
                  )}
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    setSelectedAgent(agent);
                    setEditAgentOpen(true);
                  }}
                >
                  {t('agent.actions.edit')}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  className="text-muted-foreground hover:text-destructive"
                  onClick={() => {
                    setSelectedAgent(agent);
                    setDeleteAgentOpen(true);
                  }}
                >
                  {t('common.delete')}
                </Button>
              </div>
            </CardTitle>
          </div>

          <div className="space-y-4 px-5 py-4">
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
              <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.88] px-4 py-3">
                <div className="text-[11px] uppercase tracking-[0.14em] text-[hsl(var(--ui-text-tertiary))]">
                  {t('agent.model')}
                </div>
                <div className="mt-1 break-all text-[15px] font-medium text-foreground">
                  {formatMetricValue(agent.model)}
                </div>
              </div>
              <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.88] px-4 py-3">
                <div className="text-[11px] uppercase tracking-[0.14em] text-[hsl(var(--ui-text-tertiary))]">
                  {t('agent.create.contextLimit')}
                </div>
                <div className="mt-1 text-[15px] font-medium text-foreground">
                  {formatMetricValue(agent.context_limit)}
                </div>
              </div>
              <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.88] px-4 py-3">
                <div className="text-[11px] uppercase tracking-[0.14em] text-[hsl(var(--ui-text-tertiary))]">
                  {t('agent.access.maxConcurrent')}
                </div>
                <div className="mt-1 text-[15px] font-medium text-foreground">
                  {formatMetricValue(agent.max_concurrent_tasks ?? 5)}
                </div>
              </div>
              <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.88] px-4 py-3">
                <div className="text-[11px] uppercase tracking-[0.14em] text-[hsl(var(--ui-text-tertiary))]">
                  {t('agent.create.thinkingEnabled', 'Think')}
                </div>
                <div className="mt-1 text-[15px] font-medium text-foreground">
                  {agent.thinking_enabled ? t('common.enabled', 'On') : t('common.disabled', 'Off')}
                </div>
              </div>
              <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.88] px-4 py-3">
                <div className="text-[11px] uppercase tracking-[0.14em] text-[hsl(var(--ui-text-tertiary))]">
                  {t('agent.create.supportsMultimodal')}
                </div>
                <div className="mt-1 text-[15px] font-medium text-foreground">
                  {agent.supports_multimodal ? t('common.enabled') : t('common.disabled')}
                </div>
              </div>
            </div>

            <div className="flex flex-wrap gap-2.5">
              {primaryMeta.map((item) => (
                <div
                  key={item.key}
                  className="inline-flex min-w-[168px] flex-1 items-center gap-2 rounded-[14px] border border-[hsl(var(--ui-line-soft))/0.62] bg-[hsl(var(--ui-surface-panel-muted))/0.42] px-3 py-2 text-[12px]"
                >
                  <span className="shrink-0 text-[hsl(var(--ui-text-tertiary))]">{item.label}</span>
                  <span className="truncate text-[hsl(var(--ui-text-secondary))]">{item.value}</span>
                </div>
              ))}
            </div>
          </div>

          <div className="space-y-3 border-t border-[hsl(var(--ui-line-soft))/0.68] bg-[hsl(var(--ui-surface-panel-muted))/0.22] px-5 py-4">
            <section className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.62] bg-[hsl(var(--ui-surface-panel-strong))/0.86] px-4 py-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                  <span className="h-2 w-2 rounded-full bg-[hsl(var(--semantic-extension))]" />
                  <div className="text-[12px] font-medium text-[hsl(var(--ui-text-secondary))]">
                    {t('agent.extensions.enabled')}
                  </div>
                </div>
                <div className="text-[11px] text-[hsl(var(--ui-text-tertiary))]">
                  {extensionChips.length > 0
                    ? `${extensionChips.length} ${t('agent.extensions.enabled')}`
                    : t('agent.extensions.none')}
                </div>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {extensionChips.length > 0 ? extensionChips.map((item) => (
                  <Badge
                    key={item.key}
                    variant={item.variant}
                    className="border-[hsl(var(--semantic-extension))/0.16] bg-[hsl(var(--semantic-extension))/0.08] text-[hsl(var(--semantic-extension))]"
                  >
                    {item.label}
                  </Badge>
                )) : (
                  <span className="text-[12px] text-[hsl(var(--ui-text-tertiary))]">{t('agent.extensions.none')}</span>
                )}
              </div>
            </section>

            <section className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.62] bg-[hsl(var(--ui-surface-panel-strong))/0.86] px-4 py-3">
              <div className="mb-3 flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                  <span className="h-2 w-2 rounded-full bg-[hsl(var(--status-warning-text))]" />
                  <div className="text-[12px] font-medium text-[hsl(var(--ui-text-secondary))]">
                    {t('agent.skills.assignedSkills')}
                  </div>
                </div>
                <div className="text-[11px] text-[hsl(var(--ui-text-tertiary))]">
                  {enabledSkillNames.length > 0
                    ? `${enabledSkillNames.length} ${t('agent.skills.assignedSkills')}`
                    : t('agent.skills.noSkillsAssigned')}
                </div>
              </div>
              <div className="flex flex-wrap gap-1.5">
                {skillChips.length > 0 ? skillChips.map((item) => (
                  <Badge
                    key={item.key}
                    variant="outline"
                    className={item.className}
                  >
                    {item.label}
                  </Badge>
                )) : (
                  <span className="text-[12px] text-[hsl(var(--ui-text-tertiary))]">{t('agent.skills.noSkillsAssigned')}</span>
                )}
              </div>
            </section>
          </div>

          {agent.status === 'error' && agent.last_error && (
            <div className="border-t border-[hsl(var(--ui-line-soft))/0.68] px-5 py-4">
              <div className="rounded-[18px] border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.88] px-4 py-3 text-[12px] text-status-error-text">
                {agent.last_error}
              </div>
            </div>
          )}
        </CardHeader>
      </Card>
    );
  };

  const renderDedicatedGroup = (group: DedicatedAvatarGroup) => {
    const managerLabel = group.managerAgent
      ? getDedicatedAgentDisplayName(group.managerAgent, group.managerAgent.name)
      : t('agent.manage.ungroupedManagerTitle', '未归类分组');
    const managerSummary = group.managerAgent?.description?.trim()
      || t('agent.manage.avatarSectionHint', '仅用于数字分身治理与执行，配置调整不影响常规 Agent。');
    const previewNames = group.portals
      .slice(0, 3)
      .map(item => item.portalName)
      .filter(Boolean)
      .join(' · ');

    return (
      <div key={group.managerId} className="rounded-xl border border-border/70 bg-card px-4 py-4">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-center lg:justify-between">
          <div className="min-w-0 flex-1 space-y-2">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-sm font-semibold text-foreground">{managerLabel}</span>
              {group.managerRoles.map(role => (
                <AgentTypeBadge
                  key={role}
                  type={role === 'manager' ? 'avatar_manager' : 'avatar_service'}
                />
              ))}
              <Badge variant="secondary" className="text-[11px]">
                {t('agent.manage.dedicatedGroupAvatarCount', '{{count}} 个分身', {
                  count: group.portals.length,
                })}
              </Badge>
            </div>
            <div className="text-xs text-muted-foreground line-clamp-1">
              {group.managerAgent ? `${t('agent.model', '模型')}: ${group.managerAgent.model || '-'} · ${managerSummary}` : managerSummary}
            </div>
            <div className="text-xs text-muted-foreground line-clamp-1">
              {group.portals.length > 0
                ? t('agent.manage.dedicatedGroupPreview', '包含分身：{{names}}', { names: previewNames || '-' })
                : t('agent.manage.noAvatarUnderManager', '当前管理 Agent 下还没有分身服务 Agent。')}
            </div>
          </div>
          <div className="shrink-0">
            <Button
              variant="outline"
              size="sm"
              onClick={() => navigate(`/teams/${teamId}/agent/avatar-managers/${group.managerId}`)}
            >
              {t('agent.manage.openDedicatedDetail', '打开详情页')}
            </Button>
          </div>
        </div>
      </div>
    );
  };

  const dedicatedManagerOptions = dedicatedGroups.map(group => ({
    value: group.managerId,
    label: group.managerAgent
      ? getDedicatedAgentDisplayName(group.managerAgent, group.managerAgent.name)
      : t('agent.manage.ungroupedManagerTitle', '未归类分组'),
  }));

  const filteredDedicatedGroups = dedicatedGroups.filter(group =>
    dedicatedManagerFilter === '__all__' ? true : group.managerId === dedicatedManagerFilter
  );

  useEffect(() => {
    if (dedicatedManagerFilter === '__all__') return;
    const exists = dedicatedGroups.some(group => group.managerId === dedicatedManagerFilter);
    if (!exists) {
      setDedicatedManagerFilter('__all__');
    }
  }, [dedicatedGroups, dedicatedManagerFilter]);

  const hasAnyAgents = agents.length > 0 || dedicatedGroups.length > 0;

  const openUngroupedDetail = filteredDedicatedGroups.some(group => group.managerId === UNGROUPED_MANAGER_KEY);

  if (loading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-32 w-full" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t('teamNav.agentManage')}</h2>
        <div className="flex items-center gap-2">
          {onOpenDigitalAvatar && (
            <Button variant="outline" onClick={onOpenDigitalAvatar}>
              {t('agent.manage.openAvatarCenter', '分身配置中心')}
            </Button>
          )}
          <Button onClick={() => setCreateAgentOpen(true)}>
            {t('agent.create.button')}
          </Button>
        </div>
      </div>
      {hiddenDedicatedCount > 0 && (
        <div className="rounded-md border border-border/70 bg-muted/20 px-3 py-3">
          <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
            <div className="space-y-1">
              <div className="text-sm text-muted-foreground">
                {t('agent.manage.hiddenDedicatedHint', '已隐藏 {{count}} 个专用 Agent，请展开专用分组管理。', {
                  count: hiddenDedicatedCount,
                })}
              </div>
              <div className="text-xs text-muted-foreground/80">
                {t('agent.manage.hiddenDedicatedExpandableHint', '这里默认折叠；展开后可查看数字分身专用 Agent 和生态协作专用服务 Agent。')}
              </div>
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setShowDedicatedAgents(value => !value)}
            >
              {showDedicatedAgents
                ? t('agent.manage.hideDedicatedAgents', '收起专用 Agent')
                : t('agent.manage.showDedicatedAgents', '查看专用 Agent')}
            </Button>
          </div>
        </div>
      )}

      {hasAnyAgents ? (
        <div className="space-y-6">
          <div className="space-y-3">
            <div>
              <h3 className="text-base font-semibold">{t('agent.manage.standardSectionTitle', '常规 Agent')}</h3>
              <p className="text-sm text-muted-foreground">
                {t('agent.manage.standardSectionHint', '面向团队日常对话与任务，不包含数字分身或生态协作的专用代理。')}
              </p>
            </div>
            {agents.length > 0 ? (
              <div className="grid gap-4 xl:grid-cols-2">
                {agents.map(agent => renderAgentCard(agent))}
              </div>
            ) : (
              <Card>
                <CardContent className="py-6 text-sm text-muted-foreground">
                  {t('agent.manage.noStandardAgents', '暂无常规 Agent')}
                </CardContent>
              </Card>
            )}
          </div>
          {dedicatedGroups.length > 0 && showDedicatedAgents && (
            <div className="space-y-3">
              <div className="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
                <div>
                <h3 className="text-base font-semibold">
                  {t('agent.manage.avatarSectionTitle', '专用 Agent（隔离区）')}
                </h3>
                <p className="text-sm text-muted-foreground">
                  {t('agent.manage.avatarSectionCompactHint', '这里统一收纳数字分身专用 Agent 与生态协作专用服务 Agent，避免混入常规 Agent。')}
                </p>
                </div>
                <div className="w-full md:w-72">
                  <div className="mb-1 text-xs text-muted-foreground">
                    {t('agent.manage.dedicatedFilterLabel', '按管理 Agent 查看')}
                  </div>
                  <Select value={dedicatedManagerFilter} onValueChange={setDedicatedManagerFilter}>
                    <SelectTrigger className="h-9 text-sm">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="__all__">
                        {t('agent.manage.dedicatedFilterAll', '全部管理 Agent')}
                      </SelectItem>
                      {dedicatedManagerOptions.map(option => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
                  <div className="space-y-4">
                    {filteredDedicatedGroups.map(renderDedicatedGroup)}
                  </div>
              {openUngroupedDetail && (
                <div className="text-xs text-muted-foreground">
                  {t('agent.manage.ungroupedManagerHint', '未归类分组代表当前存在分身服务 Agent，但尚未正确回挂到管理 Agent。')}
                </div>
              )}
              {ecosystemDedicatedAgents.length > 0 && (
                <div className="space-y-3">
                  <div>
                    <h4 className="text-sm font-semibold text-foreground">
                      {t('agent.manage.ecosystemDedicatedTitle', '生态协作专用服务 Agent')}
                    </h4>
                    <p className="text-xs text-muted-foreground">
                      {t('agent.manage.ecosystemDedicatedHint', '这些 Agent 只服务生态协作 Portal，不应混入常规 Agent 和团队日常对话入口。')}
                    </p>
                  </div>
                  <div className="grid gap-4 xl:grid-cols-2">
                    {ecosystemDedicatedAgents.map(agent => renderAgentCard(agent))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      ) : (
        <Card>
          <CardContent className="py-8 text-center">
            <p className="mb-4 text-muted-foreground">{t('agent.noAgent')}</p>
            <Button onClick={() => setCreateAgentOpen(true)}>
              {t('agent.create.button')}
            </Button>
          </CardContent>
        </Card>
      )}

      <CreateAgentDialog
        teamId={teamId}
        open={createAgentOpen}
        onOpenChange={setCreateAgentOpen}
        onCreated={loadAgents}
      />
      <EditAgentDialog
        agent={selectedAgent}
        open={editAgentOpen}
        onOpenChange={setEditAgentOpen}
        onUpdated={loadAgents}
      />
      <DeleteAgentDialog
        agent={selectedAgent}
        open={deleteAgentOpen}
        onOpenChange={setDeleteAgentOpen}
        onDeleted={loadAgents}
      />
      <Dialog open={defaultAgentDetailOpen} onOpenChange={setDefaultAgentDetailOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {t('agent.manage.defaultAgentDialogTitle', '默认 Agent 当前职责')}
            </DialogTitle>
          </DialogHeader>
          {defaultAgentDetailTarget && (
            <div className="space-y-4">
              <div className="space-y-1">
                <div className="text-sm font-semibold text-foreground">
                  {defaultAgentDetailTarget.name}
                </div>
                <p className="text-sm text-muted-foreground">
                  {t(
                    'agent.manage.defaultAgentDialogDescription',
                    '下面展示这个 Agent 当前在系统中承担的默认职责，均基于团队设置里的真实配置。'
                  )}
                </p>
              </div>
              <div className="space-y-3">
                {getDefaultAgentResponsibilities(defaultAgentDetailTarget).map((item) => (
                  <div key={item.key} className="rounded-lg border border-border/70 bg-muted/20 px-4 py-3">
                    <div className="text-sm font-medium text-foreground">{item.title}</div>
                    <div className="mt-1 text-sm text-muted-foreground">{item.description}</div>
                  </div>
                ))}
              </div>
              <p className="text-xs text-muted-foreground">
                {t(
                  'agent.manage.defaultAgentDialogHint',
                  '如需调整这些默认职责，请到团队设置里修改默认 Agent 配置。'
                )}
              </p>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
