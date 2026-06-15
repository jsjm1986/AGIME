import { useState, useEffect, useCallback } from 'react';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { Plus, Clock, Calendar, Play, Pause, Trash2, Check } from 'lucide-react';
import ScheduledTaskModal from './ScheduledTaskModal';
import {
  listTasks,
  deleteTask,
  pauseTask,
  resumeTask,
  runTaskNow,
  publishTask,
} from '../../services/scheduledTaskClient';
import type {
  ScheduledTaskSummary,
  ScheduledTaskParseResult,
} from '../../types/scheduledTask';
import { toastSuccess, toastError } from '../../toasts';
import { ConfirmationModal } from '../ui/ConfirmationModal';

interface ScheduledTasksViewProps {
  onClose: () => void;
}

type TFunc = ReturnType<typeof useTranslation>['t'];

const formatTime = (isoString: string | null | undefined, t: TFunc) => {
  if (!isoString) return t('neverRun');
  try {
    return new Date(isoString).toLocaleString();
  } catch {
    return isoString;
  }
};

function StatusBadge({ status, t }: { status: ScheduledTaskSummary['status']; t: TFunc }) {
  const styles: Record<string, string> = {
    active: 'bg-green-500/20 text-green-500',
    paused: 'bg-yellow-500/20 text-yellow-500',
    draft: 'bg-gray-500/20 text-gray-500',
    completed: 'bg-blue-500/20 text-blue-500',
    deleted: 'bg-red-500/20 text-red-500',
  };
  return (
    <span className={`px-2 py-0.5 rounded-full text-xs ${styles[status] || ''}`}>
      {t(`status.${status}`)}
    </span>
  );
}

function KindBadge({ kind, t }: { kind: ScheduledTaskSummary['task_kind']; t: TFunc }) {
  return (
    <span className="px-2 py-0.5 rounded-full text-xs bg-teal-500/20 text-teal-500">
      {kind === 'one_shot' ? t('parse.oneShot') : t('parse.recurring')}
    </span>
  );
}

interface TaskCardProps {
  task: ScheduledTaskSummary;
  t: TFunc;
  actionLoading: string | null;
  onEdit: (task: ScheduledTaskSummary) => void;
  onPublish: (task: ScheduledTaskSummary) => void;
  onPause: (task: ScheduledTaskSummary) => void;
  onResume: (task: ScheduledTaskSummary) => void;
  onRunNow: (task: ScheduledTaskSummary) => void;
  onDelete: (task: ScheduledTaskSummary) => void;
}

function TaskCard({
  task,
  t,
  actionLoading,
  onEdit,
  onPublish,
  onPause,
  onResume,
  onRunNow,
  onDelete,
}: TaskCardProps) {
  const busy = actionLoading === task.task_id;
  return (
    <div className="bg-background-subtle border border-border-subtle rounded-lg p-4 hover:border-border-default transition-colors">
      <div className="flex items-start justify-between mb-3">
        <div className="flex-1 min-w-0">
          <h3 className="font-medium text-text-default truncate">{task.title}</h3>
          <div className="flex items-center gap-2 mt-1.5">
            <StatusBadge status={task.status} t={t} />
            <KindBadge kind={task.task_kind} t={t} />
          </div>
        </div>
        <div className="flex items-center gap-1 ml-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onEdit(task)}
            disabled={busy}
            className="h-8 w-8 p-0"
          >
            <Calendar className="h-4 w-4" />
          </Button>
          {task.status === 'draft' && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onPublish(task)}
              disabled={busy}
              className="h-8 w-8 p-0 text-green-500 hover:text-green-400"
            >
              <Check className="h-4 w-4" />
            </Button>
          )}
          {task.status === 'active' && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onPause(task)}
              disabled={busy}
              className="h-8 w-8 p-0 text-yellow-500 hover:text-yellow-400"
            >
              <Pause className="h-4 w-4" />
            </Button>
          )}
          {task.status === 'paused' && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onResume(task)}
              disabled={busy}
              className="h-8 w-8 p-0 text-green-500 hover:text-green-400"
            >
              <Play className="h-4 w-4" />
            </Button>
          )}
          {task.status !== 'deleted' && task.status !== 'completed' && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onRunNow(task)}
              disabled={busy}
              className="h-8 w-8 p-0 text-blue-500 hover:text-blue-400"
            >
              <Play className="h-4 w-4" />
            </Button>
          )}
          {task.status !== 'deleted' && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onDelete(task)}
              disabled={busy}
              className="h-8 w-8 p-0 text-red-500 hover:text-red-400"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          )}
        </div>
      </div>

      <div className="space-y-1 text-xs text-text-muted">
        {task.next_fire_at && (
          <div className="flex items-center gap-1.5">
            <Clock className="h-3 w-3" />
            <span>{t('nextRun', { time: formatTime(task.next_fire_at, t) })}</span>
          </div>
        )}
        {task.last_fire_at ? (
          <div className="flex items-center gap-1.5">
            <Clock className="h-3 w-3" />
            <span>{t('lastRun', { time: formatTime(task.last_fire_at, t) })}</span>
          </div>
        ) : (
          <div className="flex items-center gap-1.5">
            <Clock className="h-3 w-3" />
            <span>{t('neverRun')}</span>
          </div>
        )}
      </div>
    </div>
  );
}

function TaskGroup({
  title,
  taskList,
  renderCard,
}: {
  title: string;
  taskList: ScheduledTaskSummary[];
  renderCard: (task: ScheduledTaskSummary) => ReactNode;
}) {
  if (taskList.length === 0) return null;
  return (
    <div className="space-y-3">
      <h2 className="text-sm font-medium text-text-muted uppercase tracking-wider">{title}</h2>
      <div className="space-y-2">{taskList.map((task) => renderCard(task))}</div>
    </div>
  );
}

export default function ScheduledTasksView({ onClose }: ScheduledTasksViewProps) {
  const { t } = useTranslation('scheduledTasks');
  const [tasks, setTasks] = useState<ScheduledTaskSummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [editingTask, setEditingTask] = useState<ScheduledTaskSummary | null>(null);
  const [deletingTask, setDeletingTask] = useState<ScheduledTaskSummary | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [parseResult, setParseResult] = useState<ScheduledTaskParseResult | null>(null);

  const fetchTasks = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);
      const result = await listTasks();
      setTasks(result);
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('errors.unknownFetch');
      setError(msg);
      toastError({ title: t('errors.unknownFetch'), msg });
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    fetchTasks();
  }, [fetchTasks]);

  const handleCreate = () => {
    setEditingTask(null);
    setParseResult(null);
    setIsModalOpen(true);
  };

  const handleEdit = (task: ScheduledTaskSummary) => {
    setEditingTask(task);
    setParseResult(null);
    setIsModalOpen(true);
  };

  const handleModalClose = () => {
    setIsModalOpen(false);
    setEditingTask(null);
    setParseResult(null);
  };

  const handleTaskCreated = (task: ScheduledTaskSummary) => {
    fetchTasks();
    handleModalClose();
    toastSuccess({ title: t('toasts.createSuccess'), msg: t('toasts.createSuccessMsg', { title: task.title }) });
  };

  const handleTaskUpdated = (task: ScheduledTaskSummary) => {
    fetchTasks();
    handleModalClose();
    toastSuccess({ title: t('toasts.updateSuccess'), msg: t('toasts.updateSuccessMsg', { title: task.title }) });
  };

  const handleDelete = async () => {
    if (!deletingTask) return;
    setActionLoading(deletingTask.task_id);
    try {
      await deleteTask(deletingTask.task_id);
      setTasks((prev) => prev.filter((t) => t.task_id !== deletingTask.task_id));
      toastSuccess({ title: t('toasts.deleteSuccess'), msg: t('toasts.deleteSuccessMsg', { title: deletingTask.title }) });
    } catch {
      toastError({ title: t('toasts.deleteError'), msg: t('errors.unknownDelete', { title: deletingTask.title }) });
    } finally {
      setActionLoading(null);
      setDeletingTask(null);
    }
  };

  const handlePause = async (task: ScheduledTaskSummary) => {
    setActionLoading(task.task_id);
    try {
      await pauseTask(task.task_id);
      fetchTasks();
      toastSuccess({ title: t('toasts.pauseSuccess'), msg: t('toasts.pauseSuccessMsg', { title: task.title }) });
    } catch {
      toastError({ title: t('toasts.pauseError'), msg: t('errors.unknownPause', { title: task.title }) });
    } finally {
      setActionLoading(null);
    }
  };

  const handleResume = async (task: ScheduledTaskSummary) => {
    setActionLoading(task.task_id);
    try {
      await resumeTask(task.task_id);
      fetchTasks();
      toastSuccess({ title: t('toasts.resumeSuccess'), msg: t('toasts.resumeSuccessMsg', { title: task.title }) });
    } catch {
      toastError({ title: t('toasts.resumeError'), msg: t('errors.unknownResume', { title: task.title }) });
    } finally {
      setActionLoading(null);
    }
  };

  const handleRunNow = async (task: ScheduledTaskSummary) => {
    setActionLoading(task.task_id);
    try {
      await runTaskNow(task.task_id);
      fetchTasks();
      toastSuccess({ title: t('toasts.runNowSuccess'), msg: t('toasts.runNowSuccessMsg', { title: task.title }) });
    } catch {
      toastError({ title: t('toasts.runNowError'), msg: t('errors.unknownRunNow', { title: task.title }) });
    } finally {
      setActionLoading(null);
    }
  };

  const handlePublish = async (task: ScheduledTaskSummary) => {
    setActionLoading(task.task_id);
    try {
      await publishTask(task.task_id);
      fetchTasks();
      toastSuccess({ title: t('toasts.publishSuccess'), msg: t('toasts.publishSuccessMsg', { title: task.title }) });
    } catch {
      toastError({ title: t('toasts.publishError'), msg: t('errors.unknownPublish', { title: task.title }) });
    } finally {
      setActionLoading(null);
    }
  };

  // Exclude any 'deleted' tasks from display. They aren't shown in any status
  // group, so counting them in `tasks.length` would make a task occupy the
  // list (suppressing the empty state) while rendering in no group — i.e.
  // silently vanish. Filtering here keeps the count and the rendered groups
  // consistent.
  const visibleTasks = tasks.filter((task) => task.status !== 'deleted');
  const activeTasks = visibleTasks.filter((task) => task.status === 'active');
  const pausedTasks = visibleTasks.filter((task) => task.status === 'paused');
  const draftTasks = visibleTasks.filter((task) => task.status === 'draft');
  const completedTasks = visibleTasks.filter((task) => task.status === 'completed');

  const renderCard = (task: ScheduledTaskSummary) => (
    <TaskCard
      key={task.task_id}
      task={task}
      t={t}
      actionLoading={actionLoading}
      onEdit={handleEdit}
      onPublish={handlePublish}
      onPause={handlePause}
      onResume={handleResume}
      onRunNow={handleRunNow}
      onDelete={setDeletingTask}
    />
  );

  return (
    <MainPanelLayout
      title={t('title')}
      description={t('description')}
      onClose={onClose}
    >
      <div className="p-4 space-y-6">
        {/* Create Button */}
        <Button onClick={handleCreate} className="gap-2">
          <Plus className="h-4 w-4" />
          {t('create')}
        </Button>

        {/* Content */}
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500" />
          </div>
        ) : error ? (
          <div className="text-center py-12">
            <p className="text-red-500 mb-4">{error}</p>
            <Button variant="outline" onClick={fetchTasks}>
              {t('retry')}
            </Button>
          </div>
        ) : visibleTasks.length === 0 ? (
          <div className="text-center py-12">
            <Clock className="h-12 w-12 mx-auto mb-4 text-text-muted opacity-50" />
            <h3 className="text-lg font-medium text-text-default mb-2">{t('noTasks')}</h3>
            <p className="text-text-muted text-sm">{t('noTasksDescription')}</p>
          </div>
        ) : (
          <div className="space-y-6">
            <TaskGroup title={t('card.active')} taskList={activeTasks} renderCard={renderCard} />
            <TaskGroup title={t('card.paused')} taskList={pausedTasks} renderCard={renderCard} />
            <TaskGroup title={t('card.draft')} taskList={draftTasks} renderCard={renderCard} />
            <TaskGroup
              title={t('card.completed')}
              taskList={completedTasks}
              renderCard={renderCard}
            />
          </div>
        )}
      </div>

      {/* Modal */}
      {isModalOpen && (
        <ScheduledTaskModal
          task={editingTask}
          initialParseResult={parseResult}
          onClose={handleModalClose}
          onCreated={handleTaskCreated}
          onUpdated={handleTaskUpdated}
        />
      )}

      {/* Delete Confirmation */}
      <ConfirmationModal
        isOpen={!!deletingTask}
        onCancel={() => setDeletingTask(null)}
        onConfirm={handleDelete}
        title={t('deleteConfirmTitle')}
        message={t('deleteConfirm')}
        confirmLabel={t('delete')}
        cancelLabel={t('modal.cancel')}
        confirmVariant="destructive"
      />
    </MainPanelLayout>
  );
}