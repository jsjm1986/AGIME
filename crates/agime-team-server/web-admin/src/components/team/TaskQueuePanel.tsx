import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/select';
import { SubmitTaskDialog } from '../agent/SubmitTaskDialog';
import { TaskDetailDialog } from '../agent/TaskDetailDialog';
import {
  agentApi,
  taskApi,
  TeamAgent,
  AgentTask,
} from '../../api/agent';

interface TaskQueuePanelProps {
  teamId: string;
}

export function TaskQueuePanel({ teamId }: TaskQueuePanelProps) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [tasks, setTasks] = useState<AgentTask[]>([]);
  const [loading, setLoading] = useState(true);
  const [statusFilter, setStatusFilter] = useState<string>('');
  const [submitTaskOpen, setSubmitTaskOpen] = useState(false);
  const [selectedTask, setSelectedTask] = useState<AgentTask | null>(null);
  const [taskDetailOpen, setTaskDetailOpen] = useState(false);

  const loadData = async () => {
    try {
      setLoading(true);
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

  useEffect(() => {
    loadData();
  }, [teamId, statusFilter]);

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

  const getStatusBadge = (status: string) => {
    const variants: Record<string, 'default' | 'secondary' | 'destructive' | 'outline'> = {
      pending: 'outline',
      approved: 'secondary',
      running: 'default',
      completed: 'default',
      rejected: 'destructive',
      failed: 'destructive',
      cancelled: 'secondary',
    };
    return <Badge variant={variants[status] || 'outline'}>{t(`agent.status.${status}`)}</Badge>;
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t('teamNav.taskQueue')}</h2>
        {agents.length > 0 && (
          <Button variant="outline" onClick={() => setSubmitTaskOpen(true)}>
            {t('agent.task.submit')}
          </Button>
        )}
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center justify-between">
            <span>{t('agent.taskQueue')}</span>
            <Select value={statusFilter || '__all__'} onValueChange={(v) => setStatusFilter(v === '__all__' ? '' : v)}>
              <SelectTrigger className="w-[160px] h-8 text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__all__">{t('agent.filter.all')}</SelectItem>
                <SelectItem value="pending">{t('agent.status.pending')}</SelectItem>
                <SelectItem value="approved">{t('agent.status.approved')}</SelectItem>
                <SelectItem value="running">{t('agent.status.running')}</SelectItem>
                <SelectItem value="completed">{t('agent.status.completed')}</SelectItem>
                <SelectItem value="failed">{t('agent.status.failed')}</SelectItem>
              </SelectContent>
            </Select>
          </CardTitle>
        </CardHeader>
        <CardContent>
          {loading ? (
            <div className="py-8 text-center text-muted-foreground">{t('common.loading')}</div>
          ) : tasks.length === 0 ? (
            <div className="py-8 text-center text-muted-foreground">{t('agent.noTasks')}</div>
          ) : (
            <div className="space-y-2">
              {tasks.map((task) => (
                <div
                  key={task.id}
                  role="button"
                  tabIndex={0}
                  className="flex items-center justify-between p-3 border rounded-lg hover:bg-muted/50 cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => { setSelectedTask(task); setTaskDetailOpen(true); }}
                  onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setSelectedTask(task); setTaskDetailOpen(true); } }}
                >
                  <div className="flex items-center gap-4">
                    <span className="text-sm font-mono text-muted-foreground">#{task.id.slice(0, 8)}</span>
                    <span className="font-medium">{task.task_type}</span>
                    {getStatusBadge(task.status)}
                    <span className="text-xs text-muted-foreground">
                      {new Date(task.submitted_at).toLocaleString()}
                    </span>
                  </div>
                  <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                    {task.status === 'pending' && (
                      <>
                        <Button size="sm" onClick={() => handleApprove(task.id)}>{t('agent.actions.approve')}</Button>
                        <Button size="sm" variant="outline" onClick={() => handleReject(task.id)}>{t('agent.actions.reject')}</Button>
                      </>
                    )}
                    {(task.status === 'approved' || task.status === 'running') && (
                      <Button size="sm" variant="destructive" onClick={() => handleCancel(task.id)}>{t('agent.actions.cancel')}</Button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

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
        onAction={() => { loadData(); setTaskDetailOpen(false); }}
      />
    </div>
  );
}