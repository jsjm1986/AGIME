import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useParams, useNavigate } from 'react-router-dom';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card';
import { Button } from '../components/ui/button';
import { Badge } from '../components/ui/badge';
import { Skeleton } from '../components/ui/skeleton';
import { CreateAgentDialog } from '../components/agent/CreateAgentDialog';
import { EditAgentDialog } from '../components/agent/EditAgentDialog';
import { DeleteAgentDialog } from '../components/agent/DeleteAgentDialog';
import { SubmitTaskDialog } from '../components/agent/SubmitTaskDialog';
import { TaskDetailDialog } from '../components/agent/TaskDetailDialog';
// ChatDialog deprecated - replaced by ChatPage (Phase 1 Chat Track)
// import { ChatDialog } from '../components/agent/ChatDialog';
import {
  agentApi,
  taskApi,
  TeamAgent,
  AgentTask,
  BUILTIN_EXTENSIONS,
} from '../api/agent';

export function TeamAgentPage() {
  const { t } = useTranslation();
  const { teamId } = useParams<{ teamId: string }>();
  const navigate = useNavigate();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [tasks, setTasks] = useState<AgentTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [statusFilter, setStatusFilter] = useState<string>('');

  // Dialog states
  const [createAgentOpen, setCreateAgentOpen] = useState(false);
  const [editAgentOpen, setEditAgentOpen] = useState(false);
  const [deleteAgentOpen, setDeleteAgentOpen] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(null);
  const [submitTaskOpen, setSubmitTaskOpen] = useState(false);
  const [selectedTask, setSelectedTask] = useState<AgentTask | null>(null);
  const [taskDetailOpen, setTaskDetailOpen] = useState(false);
  // ChatDialog state removed - now navigates to ChatPage

  useEffect(() => {
    if (teamId) {
      loadData();
    }
  }, [teamId, statusFilter]);

  const loadData = async () => {
    if (!teamId) return;
    setLoading(true);
    try {
      const [agentsRes, tasksRes] = await Promise.all([
        agentApi.listAgents(teamId),
        taskApi.listTasks(teamId, 1, 50, statusFilter || undefined),
      ]);
      setAgents(agentsRes.items);
      setTasks(tasksRes.items);
    } catch (error) {
      console.error('Failed to load data:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleApprove = async (taskId: string) => {
    try {
      await taskApi.approveTask(taskId);
      loadData();
    } catch (error) {
      console.error('Failed to approve task:', error);
    }
  };

  const handleReject = async (taskId: string) => {
    try {
      await taskApi.rejectTask(taskId);
      loadData();
    } catch (error) {
      console.error('Failed to reject task:', error);
    }
  };

  const handleCancel = async (taskId: string) => {
    try {
      await taskApi.cancelTask(taskId);
      loadData();
    } catch (error) {
      console.error('Failed to cancel task:', error);
    }
  };

  const handleTaskClick = (task: AgentTask) => {
    setSelectedTask(task);
    setTaskDetailOpen(true);
  };

  const handleTaskAction = () => {
    loadData();
    setTaskDetailOpen(false);
  };

  const handleOpenChat = (agent: TeamAgent) => {
    // Navigate to ChatPage with agent pre-selected
    navigate(`/teams/${teamId}/chat?agent=${agent.id}`);
  };

  const handleEditAgent = (agent: TeamAgent) => {
    setSelectedAgent(agent);
    setEditAgentOpen(true);
  };

  const handleDeleteAgent = (agent: TeamAgent) => {
    setSelectedAgent(agent);
    setDeleteAgentOpen(true);
  };

  // Get enabled extension names for display
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
      pending: 'outline',
      approved: 'secondary',
      rejected: 'destructive',
      completed: 'default',
      failed: 'destructive',
      cancelled: 'secondary',
    };
    return <Badge variant={variants[status] || 'outline'}>{t(`agent.status.${status}`)}</Badge>;
  };

  if (loading) {
    return (
      <AppShell>
        <PageHeader title={t('agent.title')} />
        <div className="p-6 space-y-4">
          <Skeleton className="h-32 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      </AppShell>
    );
  }

  return (
    <AppShell>
      <PageHeader
        title={t('agent.title')}
        actions={
          <div className="flex gap-2">
            <Button onClick={() => setCreateAgentOpen(true)}>
              {t('agent.create.button', 'Create Agent')}
            </Button>
            {agents.length > 0 && (
              <Button variant="outline" onClick={() => setSubmitTaskOpen(true)}>
                {t('agent.task.submit', 'Submit Task')}
              </Button>
            )}
          </div>
        }
      />
      <div className="p-6 space-y-6">
        {/* Agent 列表 */}
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
                      <Button size="sm" variant="outline" onClick={() => handleEditAgent(agent)}>
                        {t('agent.actions.edit')}
                      </Button>
                      <Button size="sm" variant="outline" onClick={() => handleDeleteAgent(agent)}>
                        {t('common.delete')}
                      </Button>
                      <Button size="sm" onClick={() => handleOpenChat(agent)}>
                        {t('agent.chat.button', 'Chat')}
                      </Button>
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
                    {/* 基本信息 */}
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
                    {/* LLM 参数 */}
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

                    {/* 扩展信息 */}
                    <div className="border-t pt-4">
                      <span className="text-sm text-muted-foreground">{t('agent.extensions.enabled')}:</span>
                      <div className="flex flex-wrap gap-1 mt-2">
                        {getEnabledExtensionNames(agent).map((name) => (
                          <Badge key={name} variant="secondary" className="text-xs">
                            {name}
                          </Badge>
                        ))}
                        {agent.custom_extensions?.filter(e => e.enabled).map((ext) => (
                          <Badge key={ext.name} variant="outline" className="text-xs">
                            {ext.name}
                          </Badge>
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
                {t('agent.create.button', 'Create Agent')}
              </Button>
            </CardContent>
          </Card>
        )}

        {/* 任务队列 */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center justify-between">
              <span>{t('agent.taskQueue')}</span>
              <select
                className="text-sm border rounded px-2 py-1"
                value={statusFilter}
                onChange={(e) => setStatusFilter(e.target.value)}
              >
                <option value="">{t('agent.filter.all')}</option>
                <option value="pending">{t('agent.status.pending')}</option>
                <option value="approved">{t('agent.status.approved')}</option>
                <option value="running">{t('agent.status.running')}</option>
                <option value="completed">{t('agent.status.completed')}</option>
                <option value="failed">{t('agent.status.failed')}</option>
              </select>
            </CardTitle>
          </CardHeader>
          <CardContent>
            {tasks.length === 0 ? (
              <div className="py-8 text-center text-muted-foreground">
                {t('agent.noTasks')}
              </div>
            ) : (
              <div className="space-y-2">
                {tasks.map((task) => (
                  <div
                    key={task.id}
                    className="flex items-center justify-between p-3 border rounded-lg hover:bg-muted/50 cursor-pointer transition-colors"
                    onClick={() => handleTaskClick(task)}
                  >
                    <div className="flex items-center gap-4">
                      <span className="text-sm font-mono text-muted-foreground">
                        #{task.id.slice(0, 8)}
                      </span>
                      <span className="font-medium">{task.task_type}</span>
                      {getStatusBadge(task.status)}
                      <span className="text-xs text-muted-foreground">
                        {new Date(task.submitted_at).toLocaleString()}
                      </span>
                    </div>
                    <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                      {task.status === 'pending' && (
                        <>
                          <Button
                            size="sm"
                            onClick={() => handleApprove(task.id)}
                          >
                            {t('agent.actions.approve')}
                          </Button>
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => handleReject(task.id)}
                          >
                            {t('agent.actions.reject')}
                          </Button>
                        </>
                      )}
                      {(task.status === 'approved' || task.status === 'running') && (
                        <Button
                          size="sm"
                          variant="destructive"
                          onClick={() => handleCancel(task.id)}
                        >
                          {t('agent.actions.cancel')}
                        </Button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Dialogs */}
      {teamId && (
        <>
          <CreateAgentDialog
            teamId={teamId}
            open={createAgentOpen}
            onOpenChange={setCreateAgentOpen}
            onCreated={loadData}
          />
          <EditAgentDialog
            agent={selectedAgent}
            open={editAgentOpen}
            onOpenChange={setEditAgentOpen}
            onUpdated={loadData}
          />
          <DeleteAgentDialog
            agent={selectedAgent}
            open={deleteAgentOpen}
            onOpenChange={setDeleteAgentOpen}
            onDeleted={loadData}
          />
          <SubmitTaskDialog
            teamId={teamId}
            agents={agents}
            open={submitTaskOpen}
            onOpenChange={setSubmitTaskOpen}
            onSubmitted={loadData}
          />
          <TaskDetailDialog
            task={selectedTask}
            open={taskDetailOpen}
            onOpenChange={setTaskDetailOpen}
            onAction={handleTaskAction}
          />
          {/* ChatDialog removed - replaced by ChatPage (Phase 1 Chat Track) */}
        </>
      )}
    </AppShell>
  );
}