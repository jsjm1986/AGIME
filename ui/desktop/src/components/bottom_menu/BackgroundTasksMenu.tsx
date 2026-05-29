/**
 * BackgroundTasksMenu - a lightweight popover in the chat toolbar that lists the
 * user's long-running background tasks for the current session, shows live
 * status, and lets the user cancel a running task or expand one to watch its
 * live harness-event stream.
 *
 * Intentionally minimally invasive: it lives in the existing chat composer
 * toolbar (no new route), and reads/writes task state exclusively through the
 * {@link useTasks}/{@link useTask} hooks over the {@link taskManager} singleton.
 */

import { useState } from 'react';
import { ListChecks, Loader2, X } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Button } from '../ui/button';
import { useTasks, useTask } from '../../hooks/useTasks';
import { UserTaskSnapshot, UserTaskStatus, isTerminalStatus } from '../../services/taskClient';
import { describeHarnessEnvelope } from '../../utils/harnessControl';
import { cn } from '../../utils';

interface BackgroundTasksMenuProps {
  sessionId: string;
}

const STATUS_LABEL: Record<UserTaskStatus, string> = {
  pending: '排队中',
  running: '运行中',
  completed: '已完成',
  failed: '失败',
  cancelled: '已取消',
};

const STATUS_DOT: Record<UserTaskStatus, string> = {
  pending: 'bg-amber-400',
  running: 'bg-blue-500',
  completed: 'bg-green-500',
  failed: 'bg-red-500',
  cancelled: 'bg-neutral-400',
};

export function BackgroundTasksMenu({ sessionId }: BackgroundTasksMenuProps) {
  const { tasks, cancel } = useTasks(sessionId);
  const [isOpen, setIsOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const sessionTasks = tasks.filter((t) => t.session_id === sessionId);
  const activeCount = sessionTasks.filter(
    (t) => !isTerminalStatus(t.status)
  ).length;

  return (
    <DropdownMenu open={isOpen} onOpenChange={setIsOpen}>
      <DropdownMenuTrigger asChild>
        <button
          className="flex items-center cursor-pointer [&_svg]:size-4 text-text-default/70 hover:text-text-default text-xs"
          title="后台长程任务"
        >
          <ListChecks className="mr-1 h-4 w-4" />
          {activeCount > 0 && <span>{activeCount}</span>}
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="center" className="w-80">
        <div className="px-3 py-2 text-sm font-medium text-text-default border-b border-border-default">
          后台任务
        </div>
        <div className="max-h-[420px] overflow-y-auto">
          {sessionTasks.length === 0 ? (
            <div className="px-3 py-6 text-center text-sm text-text-default/60">
              暂无后台任务
            </div>
          ) : (
            sessionTasks.map((task) => (
              <TaskRow
                key={task.task_id}
                task={task}
                expanded={expandedId === task.task_id}
                onToggle={() =>
                  setExpandedId((prev) =>
                    prev === task.task_id ? null : task.task_id
                  )
                }
                onCancel={() => cancel(task.task_id)}
              />
            ))
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

interface TaskRowProps {
  task: UserTaskSnapshot;
  expanded: boolean;
  onToggle: () => void;
  onCancel: () => void;
}

function TaskRow({ task, expanded, onToggle, onCancel }: TaskRowProps) {
  const running = !isTerminalStatus(task.status);

  return (
    <div className="border-b border-border-default/50 last:border-b-0">
      <div
        className="flex items-start gap-2 px-3 py-2 hover:bg-background-hover cursor-pointer"
        onClick={onToggle}
      >
        <span
          className={cn(
            'mt-1 h-2 w-2 flex-shrink-0 rounded-full',
            STATUS_DOT[task.status]
          )}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm text-text-default">
            {task.prompt_preview || '(无内容)'}
          </div>
          <div className="mt-0.5 flex items-center gap-1.5 text-xs text-text-default/60">
            {running && <Loader2 className="h-3 w-3 animate-spin" />}
            <span>{STATUS_LABEL[task.status]}</span>
          </div>
        </div>
        {running && (
          <Button
            type="button"
            variant="ghost"
            size="xs"
            onClick={(e) => {
              e.stopPropagation();
              onCancel();
            }}
            className="flex-shrink-0 text-text-default/60 hover:text-red-500"
            title="取消任务"
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        )}
      </div>
      {expanded && <TaskDetail taskId={task.task_id} error={task.error} />}
    </div>
  );
}

function TaskDetail({
  taskId,
  error,
}: {
  taskId: string;
  error: string | null;
}) {
  const state = useTask(taskId);
  const events = state?.recentEvents ?? [];
  const lines = events
    .map(describeHarnessEnvelope)
    .filter((line): line is string => line != null)
    .slice(-12);

  return (
    <div className="bg-background-muted/40 px-3 py-2">
      {error && (
        <div className="mb-2 rounded bg-red-500/10 px-2 py-1 text-xs text-red-500">
          {error}
        </div>
      )}
      {lines.length === 0 ? (
        <div className="text-xs text-text-default/50">
          {state?.attached ? '正在等待事件…' : '暂无事件记录'}
        </div>
      ) : (
        <ul className="space-y-0.5">
          {lines.map((line, idx) => (
            <li
              key={idx}
              className="truncate font-mono text-[11px] text-text-default/70"
            >
              {line}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
