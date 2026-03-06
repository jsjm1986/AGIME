import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Skeleton } from '../ui/skeleton';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { StatusBadge, AGENT_STATUS_MAP } from '../ui/status-badge';
import { CreateAgentDialog } from '../agent/CreateAgentDialog';
import { EditAgentDialog } from '../agent/EditAgentDialog';
import { DeleteAgentDialog } from '../agent/DeleteAgentDialog';
import {
  agentApi,
  TeamAgent,
  BUILTIN_EXTENSIONS,
} from '../../api/agent';
import { portalApi } from '../../api/portal';
import {
  UNGROUPED_MANAGER_KEY,
  buildDedicatedAvatarGrouping,
  type DedicatedAvatarGroup,
} from './agentIsolation';

interface AgentManagePanelProps {
  teamId: string;
  onOpenChat?: (agent: TeamAgent) => void;
  onOpenDigitalAvatar?: () => void;
}

export function AgentManagePanel({ teamId, onOpenChat, onOpenDigitalAvatar }: AgentManagePanelProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [dedicatedGroups, setDedicatedGroups] = useState<DedicatedAvatarGroup[]>([]);
  const [hiddenDedicatedCount, setHiddenDedicatedCount] = useState(0);
  const [showDedicatedAgents, setShowDedicatedAgents] = useState(false);
  const [dedicatedManagerFilter, setDedicatedManagerFilter] = useState('__all__');
  const [loading, setLoading] = useState(true);

  const [createAgentOpen, setCreateAgentOpen] = useState(false);
  const [editAgentOpen, setEditAgentOpen] = useState(false);
  const [deleteAgentOpen, setDeleteAgentOpen] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(null);

  const loadAgents = async () => {
    try {
      setLoading(true);
      const [agentResult, avatarResult] = await Promise.all([
        agentApi.listAgents(teamId),
        portalApi.list(teamId, 1, 200, 'avatar'),
      ]);
      const grouping = buildDedicatedAvatarGrouping(agentResult.items || [], avatarResult.items || []);
      setAgents(grouping.generalAgents);
      setDedicatedGroups(grouping.dedicatedGroups);
      setHiddenDedicatedCount(grouping.hiddenDedicatedCount);
    } catch (error) {
      console.error('Failed to load agents:', error);
      setAgents([]);
      setDedicatedGroups([]);
      setHiddenDedicatedCount(0);
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

  const getStatusBadge = (status: string) => (
    <StatusBadge status={AGENT_STATUS_MAP[status] || 'neutral'}>
      {t(`agent.status.${status}`)}
    </StatusBadge>
  );

  const renderAgentCard = (agent: TeamAgent, roles: Array<'manager' | 'service'> = []) => {
    const enabledExtensionNames = getEnabledExtensionNames(agent);
    const enabledCustomExtensions = agent.custom_extensions?.filter(e => e.enabled) || [];

    return (
      <Card key={agent.id}>
        <CardHeader>
          <CardTitle className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span>{agent.name}</span>
              {roles.map(role => (
                <Badge key={role} variant="outline" className="text-[11px]">
                  {role === 'manager'
                    ? t('agent.manage.avatarRoleManager', '分身管理 Agent')
                    : t('agent.manage.avatarRoleService', '分身服务 Agent')}
                </Badge>
              ))}
              {getStatusBadge(agent.status)}
            </div>
            <div className="flex items-center gap-2">
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
                variant="outline"
                onClick={() => {
                  setSelectedAgent(agent);
                  setDeleteAgentOpen(true);
                }}
              >
                {t('common.delete')}
              </Button>
              {onOpenChat && (
                <Button size="sm" onClick={() => onOpenChat(agent)}>
                  {t('agent.chat.button')}
                </Button>
              )}
            </div>
          </CardTitle>
          {agent.description && (
            <p className="text-sm text-muted-foreground">{agent.description}</p>
          )}
          {agent.status === 'error' && agent.last_error && (
            <p className="mt-1 text-sm text-destructive">{agent.last_error}</p>
          )}
        </CardHeader>
        <CardContent>
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-4 text-sm md:grid-cols-4">
              <div>
                <span className="text-muted-foreground">{t('agent.create.apiFormat')}:</span>
                <span className="ml-2">{agent.api_format || '-'}</span>
              </div>
              <div>
                <span className="text-muted-foreground">{t('agent.model')}:</span>
                <span className="ml-2">{agent.model || '-'}</span>
              </div>
              <div>
                <span className="text-muted-foreground">{t('agent.access.allowedGroups')}:</span>
                <span className="ml-2">
                  {agent.allowed_groups?.length ? agent.allowed_groups.length : t('agent.access.title')}
                </span>
              </div>
              <div>
                <span className="text-muted-foreground">{t('agent.access.maxConcurrent')}:</span>
                <span className="ml-2">{agent.max_concurrent_tasks || 5}</span>
              </div>
            </div>

            {(agent.temperature != null || agent.max_tokens != null || agent.context_limit != null) && (
              <div className="grid grid-cols-2 gap-4 text-sm md:grid-cols-3">
                {agent.temperature != null && (
                  <div>
                    <span className="text-muted-foreground">{t('agent.create.temperature')}:</span>
                    <span className="ml-2">{agent.temperature}</span>
                  </div>
                )}
                {agent.max_tokens != null && (
                  <div>
                    <span className="text-muted-foreground">{t('agent.create.maxTokens')}:</span>
                    <span className="ml-2">{agent.max_tokens}</span>
                  </div>
                )}
                {agent.context_limit != null && (
                  <div>
                    <span className="text-muted-foreground">{t('agent.create.contextLimit')}:</span>
                    <span className="ml-2">{agent.context_limit}</span>
                  </div>
                )}
              </div>
            )}

            {agent.api_url && (
              <div className="text-sm">
                <span className="text-muted-foreground">{t('agent.create.apiUrl')}:</span>
                <span className="ml-2">{agent.api_url}</span>
              </div>
            )}

            <div className="border-t pt-4">
              <span className="text-sm text-muted-foreground">{t('agent.extensions.enabled')}:</span>
              <div className="mt-2 flex flex-wrap gap-1">
                {enabledExtensionNames.map((name) => (
                  <Badge key={name} variant="secondary" className="text-xs">{name}</Badge>
                ))}
                {enabledCustomExtensions.map((ext) => (
                  <Badge key={ext.name} variant="outline" className="text-xs">{ext.name}</Badge>
                ))}
                {enabledExtensionNames.length === 0 && enabledCustomExtensions.length === 0 && (
                  <span className="text-xs text-muted-foreground">{t('agent.extensions.none')}</span>
                )}
              </div>
            </div>
          </div>
        </CardContent>
      </Card>
    );
  };

  const renderDedicatedGroup = (group: DedicatedAvatarGroup) => {
    const managerLabel = group.managerAgent?.name || t('agent.manage.ungroupedManagerTitle', '未归类分组');
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
                <Badge key={role} variant="outline" className="text-[11px]">
                  {role === 'manager'
                    ? t('agent.manage.avatarRoleManager', '分身管理 Agent')
                    : t('agent.manage.avatarRoleService', '分身服务 Agent')}
                </Badge>
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
    label: group.managerAgent?.name || t('agent.manage.ungroupedManagerTitle', '未归类分组'),
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
                {t('agent.manage.hiddenDedicatedHint', '已隐藏 {{count}} 个数字分身专用 Agent，请到数字分身频道管理。', {
                  count: hiddenDedicatedCount,
                })}
              </div>
              <div className="text-xs text-muted-foreground/80">
                {t('agent.manage.hiddenDedicatedExpandableHint', '这里默认折叠，你也可以展开专用 Agent 目录，再进入独立页面查看和修改。')}
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
                {t('agent.manage.standardSectionHint', '面向团队日常对话与任务，不包含数字分身专用代理。')}
              </p>
            </div>
            {agents.length > 0 ? (
              <div className="space-y-4">
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
                  {t('agent.manage.avatarSectionTitle', '数字分身 Agent（隔离区）')}
                </h3>
                <p className="text-sm text-muted-foreground">
                  {t('agent.manage.avatarSectionCompactHint', '这里只展示管理 Agent 目录。点击后进入独立页面查看该管理组下的全部分身与配置。')}
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
    </div>
  );
}
