import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useParams, useNavigate } from 'react-router-dom';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { Card, CardContent } from '../components/ui/card';
import { CardHeader, CardTitle } from '../components/ui/card';
import { Button } from '../components/ui/button';
import { Skeleton } from '../components/ui/skeleton';
import { StatusBadge, TASK_STATUS_MAP } from '../components/ui/status-badge';
import { AgentTypeBadge, resolveAgentVisualType } from '../components/agent/AgentTypeBadge';
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
import { AgentAvatar } from '../components/agent/AvatarPicker';
import { formatDateTime } from '../utils/format';

// Agent status visual mapping
const STATUS_ACCENT: Record<string, string> = {
  idle: 'bg-zinc-400 dark:bg-zinc-600',
  running: 'bg-emerald-500',
  paused: 'bg-amber-500',
  error: 'bg-rose-500',
};
const STATUS_RING: Record<string, string> = {
  idle: 'ring-zinc-300 dark:ring-zinc-600',
  running: 'ring-emerald-400 animate-pulse',
  paused: 'ring-amber-400',
  error: 'ring-rose-400',
};
const STATUS_DOT: Record<string, string> = {
  idle: 'bg-zinc-400',
  running: 'bg-emerald-500 animate-pulse',
  paused: 'bg-amber-500',
  error: 'bg-rose-500',
};
const AVATAR_BG: Record<string, string> = {
  idle: 'bg-zinc-100 text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300',
  running: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300',
  paused: 'bg-amber-50 text-amber-700 dark:bg-amber-950 dark:text-amber-300',
  error: 'bg-rose-50 text-rose-700 dark:bg-rose-950 dark:text-rose-300',
};

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

  const getEnabledSkillNames = (agent: TeamAgent) =>
    (agent.assigned_skills || [])
      .filter(skill => skill.enabled)
      .map(skill => skill.name);

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
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
            {agents.map((agent) => {
              const extNames = getEnabledExtensionNames(agent);
              const skillNames = getEnabledSkillNames(agent);
              const customExts = agent.custom_extensions?.filter(e => e.enabled) || [];
              const totalCapabilities = extNames.length + customExts.length + skillNames.length;

              return (
                <div
                  key={agent.id}
                  className="group relative rounded-md border bg-card overflow-hidden transition-colors hover:bg-accent/20"
                >
                  {/* Status accent bar */}
                  <div className={`h-1 ${STATUS_ACCENT[agent.status] || STATUS_ACCENT.idle}`} />

                  {/* Avatar + identity */}
                  <div className="flex flex-col items-center pt-5 pb-3 px-4">
                    <div className={`w-12 h-12 rounded-full flex items-center justify-center ring-2 ring-offset-2 ring-offset-card ${STATUS_RING[agent.status] || STATUS_RING.idle} ${AVATAR_BG[agent.status] || AVATAR_BG.idle}`}>
                      <AgentAvatar avatar={agent.avatar} name={agent.name} className="w-12 h-12" iconSize="w-5 h-5" />
                    </div>
                    <h3 className="mt-2.5 text-[14px] font-semibold leading-tight text-center">{agent.name}</h3>
                    <div className="mt-1">
                      <AgentTypeBadge type={resolveAgentVisualType(agent)} />
                    </div>
                    {agent.model && (
                      <span className="mt-1 text-caption text-muted-foreground/70 font-mono">{agent.model}</span>
                    )}
                    <div className="flex items-center gap-1.5 mt-1.5">
                      <span className={`w-1.5 h-1.5 rounded-full ${STATUS_DOT[agent.status] || STATUS_DOT.idle}`} />
                      <span className="text-caption text-muted-foreground/60">{t(`agent.status.${agent.status}`)}</span>
                    </div>
                  </div>

                  {/* Description */}
                  <div className="px-4 pb-3">
                    {agent.description ? (
                      <p className="text-[12px] text-muted-foreground text-center line-clamp-2 leading-relaxed">{agent.description}</p>
                    ) : (
                      <p className="text-[12px] text-muted-foreground/40 text-center italic">{t('agent.noDescription')}</p>
                    )}
                  </div>

                  {/* Skills / extensions */}
                  <div className="px-4 pb-3">
                    <div className="flex flex-wrap justify-center gap-1">
                      {extNames.slice(0, 4).map((name) => (
                        <span key={name} className="text-micro px-1.5 py-0.5 rounded bg-muted/80 text-muted-foreground">{name}</span>
                      ))}
                      {customExts.slice(0, 2).map((ext) => (
                        <span key={ext.name} className="text-micro px-1.5 py-0.5 rounded border border-border text-muted-foreground">{ext.name}</span>
                      ))}
                      {skillNames.slice(0, 2).map((name) => (
                        <span key={name} className="text-micro px-1.5 py-0.5 rounded border border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900 dark:bg-amber-950/30 dark:text-amber-300">{name}</span>
                      ))}
                      {totalCapabilities > 8 && (
                        <span className="text-micro px-1.5 py-0.5 text-muted-foreground/50">+{totalCapabilities - 8}</span>
                      )}
                      {totalCapabilities === 0 && (
                        <span className="text-micro text-muted-foreground/40">{t('agent.extensions.none')}</span>
                      )}
                    </div>
                  </div>

                  {/* Error banner */}
                  {agent.status === 'error' && agent.last_error && (
                    <div className="mx-4 mb-3 px-2 py-1.5 rounded bg-rose-50 dark:bg-rose-950/30 text-caption text-rose-600 dark:text-rose-400 line-clamp-1">
                      {agent.last_error}
                    </div>
                  )}

                  {/* Actions */}
                  <div className="flex items-center border-t border-border/50 divide-x divide-border/50">
                    <button
                      onClick={() => handleOpenChat(agent)}
                      className="flex-1 py-2 text-[12px] font-medium text-center hover:bg-accent/40 transition-colors"
                    >
                      {t('agent.chat.button', 'Chat')}
                    </button>
                    <button
                      onClick={() => handleEditAgent(agent)}
                      className="flex-1 py-2 text-[12px] text-muted-foreground text-center hover:bg-accent/40 transition-colors"
                    >
                      {t('agent.actions.edit')}
                    </button>
                    <button
                      onClick={() => handleDeleteAgent(agent)}
                      className="px-3 py-2 text-[12px] text-muted-foreground/50 text-center hover:bg-destructive/10 hover:text-destructive transition-colors"
                    >
                      {t('common.delete')}
                    </button>
                  </div>
                </div>
              );
            })}
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
                      <StatusBadge status={TASK_STATUS_MAP[task.status]}>{t(`agent.status.${task.status}`)}</StatusBadge>
                      <span className="text-xs text-muted-foreground">
                        {formatDateTime(task.submitted_at)}
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
