import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Skeleton } from '../ui/skeleton';
import { CreateAgentDialog } from '../agent/CreateAgentDialog';
import { EditAgentDialog } from '../agent/EditAgentDialog';
import { DeleteAgentDialog } from '../agent/DeleteAgentDialog';
import {
  agentApi,
  TeamAgent,
  BUILTIN_EXTENSIONS,
} from '../../api/agent';

interface AgentManagePanelProps {
  teamId: string;
  onOpenChat?: (agent: TeamAgent) => void;
}

export function AgentManagePanel({ teamId, onOpenChat }: AgentManagePanelProps) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [loading, setLoading] = useState(true);

  const [createAgentOpen, setCreateAgentOpen] = useState(false);
  const [editAgentOpen, setEditAgentOpen] = useState(false);
  const [deleteAgentOpen, setDeleteAgentOpen] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(null);

  const loadAgents = async () => {
    try {
      setLoading(true);
      const res = await agentApi.listAgents(teamId);
      setAgents(res.items);
    } catch (error) {
      console.error('Failed to load agents:', error);
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

  const getStatusBadge = (status: string) => {
    const variants: Record<string, 'default' | 'secondary' | 'destructive' | 'outline'> = {
      idle: 'secondary',
      running: 'default',
      paused: 'outline',
      error: 'destructive',
    };
    return <Badge variant={variants[status] || 'outline'}>{t(`agent.status.${status}`)}</Badge>;
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
        <Button onClick={() => setCreateAgentOpen(true)}>
          {t('agent.create.button')}
        </Button>
      </div>

      {agents.length > 0 ? (
        <div className="space-y-4">
          {agents.map((agent) => (
            <Card key={agent.id}>
              <CardHeader>
                <CardTitle className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span>{agent.name}</span>
                    {getStatusBadge(agent.status)}
                  </div>
                  <div className="flex items-center gap-2">
                    <Button size="sm" variant="outline" onClick={() => { setSelectedAgent(agent); setEditAgentOpen(true); }}>
                      {t('agent.actions.edit')}
                    </Button>
                    <Button size="sm" variant="outline" onClick={() => { setSelectedAgent(agent); setDeleteAgentOpen(true); }}>
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
                  <p className="text-sm text-destructive mt-1">{agent.last_error}</p>
                )}
              </CardHeader>
              <CardContent>
                <div className="space-y-4">
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
                    <div>
                      <span className="text-muted-foreground">{t('agent.create.apiFormat')}:</span>
                      <span className="ml-2">{agent.api_format || '-'}</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground">{t('agent.model')}:</span>
                      <span className="ml-2">{agent.model || '-'}</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground">{t('agent.access.mode')}:</span>
                      <span className="ml-2">{t(`agent.access.${agent.access_mode || 'all'}`)}</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground">{t('agent.access.maxConcurrent')}:</span>
                      <span className="ml-2">{agent.max_concurrent_tasks || 5}</span>
                    </div>
                  </div>

                  {(agent.temperature != null || agent.max_tokens != null || agent.context_limit != null) && (
                    <div className="grid grid-cols-2 md:grid-cols-3 gap-4 text-sm">
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
                    <div className="flex flex-wrap gap-1 mt-2">
                      {getEnabledExtensionNames(agent).map((name) => (
                        <Badge key={name} variant="secondary" className="text-xs">{name}</Badge>
                      ))}
                      {agent.custom_extensions?.filter(e => e.enabled).map((ext) => (
                        <Badge key={ext.name} variant="outline" className="text-xs">{ext.name}</Badge>
                      ))}
                      {getEnabledExtensionNames(agent).length === 0 &&
                       (!agent.custom_extensions || agent.custom_extensions.filter(e => e.enabled).length === 0) && (
                        <span className="text-xs text-muted-foreground">{t('agent.extensions.none')}</span>
                      )}
                    </div>
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : (
        <Card>
          <CardContent className="py-8 text-center">
            <p className="text-muted-foreground mb-4">{t('agent.noAgent')}</p>
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