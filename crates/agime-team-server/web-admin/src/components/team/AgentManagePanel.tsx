import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Skeleton } from '../ui/skeleton';
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
import { splitGeneralAndDedicatedAgents } from './agentIsolation';

interface AgentManagePanelProps {
  teamId: string;
  onOpenChat?: (agent: TeamAgent) => void;
  onOpenDigitalAvatar?: () => void;
}

export function AgentManagePanel({ teamId, onOpenChat, onOpenDigitalAvatar }: AgentManagePanelProps) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [hiddenDedicatedCount, setHiddenDedicatedCount] = useState(0);
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
      const allAgents = agentResult.items || [];
      const avatars = avatarResult.items || [];
      const { generalAgents, dedicatedAgentIds } = splitGeneralAndDedicatedAgents(allAgents, avatars);
      setAgents(generalAgents);
      setHiddenDedicatedCount(dedicatedAgentIds.size);
    } catch (error) {
      console.error('Failed to load agents:', error);
      setAgents([]);
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

  const renderAgentCard = (agent: TeamAgent) => {
    const enabledExtensionNames = getEnabledExtensionNames(agent);
    const enabledCustomExtensions = agent.custom_extensions?.filter(e => e.enabled) || [];

    return (
      <Card key={agent.id}>
        <CardHeader>
          <CardTitle className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span>{agent.name}</span>
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
        <div className="rounded-md border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground">
          {t('agent.manage.hiddenDedicatedHint', '已隐藏 {{count}} 个数字分身专用 Agent，请到数字分身频道管理。', {
            count: hiddenDedicatedCount,
          })}
        </div>
      )}

      {agents.length > 0 ? (
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
                {agents.map(renderAgentCard)}
              </div>
            ) : (
              <Card>
                <CardContent className="py-6 text-sm text-muted-foreground">
                  {t('agent.manage.noStandardAgents', '暂无常规 Agent')}
                </CardContent>
              </Card>
            )}
          </div>
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
