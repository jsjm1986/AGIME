/**
 * useTasks / useTask - React hooks over the {@link taskManager} singleton.
 *
 * Mirrors {@link useChatStream}: the hook only subscribes to the singleton and
 * mirrors its state into React, so task streams keep running across component
 * unmounts and page navigation. All mutating actions proxy to the singleton.
 */

import { useCallback, useEffect, useState } from 'react';
import { taskManager, TaskLiveState } from '../services/TaskManager';
import { UserTaskSnapshot } from '../services/taskClient';

export interface UseTasksReturn {
  tasks: UserTaskSnapshot[];
  /** Submit a new background task for a session. Resolves to the task id. */
  submit: (sessionId: string, prompt: string) => Promise<string>;
  /** Cancel a running task. */
  cancel: (taskId: string) => Promise<void>;
  /** Re-fetch the task list from the backend (optionally for one session). */
  refresh: (sessionId?: string) => Promise<void>;
}

/**
 * Subscribe to the full task list (across all sessions), newest first. On
 * mount, refreshes the list from the backend for the given session (or all).
 */
export function useTasks(sessionId?: string): UseTasksReturn {
  const [tasks, setTasks] = useState<UserTaskSnapshot[]>(() =>
    taskManager.snapshotList()
  );

  useEffect(() => {
    const unsubscribe = taskManager.subscribeList(setTasks);
    void taskManager.refresh(sessionId).catch(() => {
      // A failed refresh leaves the current (possibly empty) list in place;
      // the live stream still updates state once tasks are submitted.
    });
    return unsubscribe;
  }, [sessionId]);

  const submit = useCallback(
    (sid: string, prompt: string) => taskManager.submit(sid, prompt),
    []
  );

  const cancel = useCallback((taskId: string) => taskManager.cancel(taskId), []);

  const refresh = useCallback(
    (sid?: string) => taskManager.refresh(sid),
    []
  );

  return { tasks, submit, cancel, refresh };
}

/**
 * Subscribe to a single task's live state and attach to its SSE stream while
 * the hook is mounted. Detaching on unmount is intentional: the singleton keeps
 * the snapshot, and a later mount resumes the stream from the last seen seq.
 */
export function useTask(taskId: string | undefined): TaskLiveState | undefined {
  const [state, setState] = useState<TaskLiveState | undefined>(() =>
    taskId ? taskManager.getTaskState(taskId) : undefined
  );

  useEffect(() => {
    if (!taskId) {
      setState(undefined);
      return;
    }
    const unsubscribe = taskManager.subscribeTask(taskId, setState);
    taskManager.attach(taskId);
    return unsubscribe;
  }, [taskId]);

  return state;
}
