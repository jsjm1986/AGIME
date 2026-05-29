/**
 * TaskManager - global singleton tracking user-level long-running tasks.
 *
 * Mirrors the ChatStreamManager pattern: a singleton holds task state
 * independent of React component lifecycle, and components subscribe for
 * updates. This lets a background task keep streaming even while the user
 * navigates away from the chat that submitted it.
 *
 * Responsibilities:
 * - submit / list / cancel proxied to {@link taskClient}
 * - attach to a task's live SSE stream and fold events into a per-task state
 * - resume an attach from the last seen seq if the connection drops
 */

import {
  UserTaskSnapshot,
  UserTaskEvent,
  UserTaskStatus,
  isTerminalStatus,
  submitTask,
  listTasks,
  getTask,
  cancelTask,
  streamTask,
} from './taskClient';

export interface TaskLiveState {
  snapshot: UserTaskSnapshot;
  /** Most recent envelope kinds seen, newest last (bounded). */
  recentEvents: UserTaskEvent[];
  /** Highest seq observed, used to resume an interrupted attach. */
  lastSeq: number;
  /** Whether a live attach stream is currently connected. */
  attached: boolean;
}

export type TaskListSubscriber = (tasks: UserTaskSnapshot[]) => void;
export type TaskSubscriber = (state: TaskLiveState) => void;

const MAX_RECENT_EVENTS = 200;

class TaskManager {
  private static instance: TaskManager;

  private tasks: Map<string, TaskLiveState> = new Map();
  private listSubscribers: Set<TaskListSubscriber> = new Set();
  private taskSubscribers: Map<string, Set<TaskSubscriber>> = new Map();
  private attachControllers: Map<string, AbortController> = new Map();

  private constructor() {}

  static getInstance(): TaskManager {
    if (!TaskManager.instance) {
      TaskManager.instance = new TaskManager();
    }
    return TaskManager.instance;
  }

  // --- subscriptions ------------------------------------------------------

  /** Subscribe to the full task list (across all sessions). */
  subscribeList(callback: TaskListSubscriber): () => void {
    this.listSubscribers.add(callback);
    callback(this.snapshotList());
    return () => {
      this.listSubscribers.delete(callback);
    };
  }

  /** Subscribe to a single task's live state. */
  subscribeTask(taskId: string, callback: TaskSubscriber): () => void {
    if (!this.taskSubscribers.has(taskId)) {
      this.taskSubscribers.set(taskId, new Set());
    }
    this.taskSubscribers.get(taskId)!.add(callback);
    const current = this.tasks.get(taskId);
    if (current) {
      callback(current);
    }
    return () => {
      this.taskSubscribers.get(taskId)?.delete(callback);
      if (this.taskSubscribers.get(taskId)?.size === 0) {
        this.taskSubscribers.delete(taskId);
      }
    };
  }

  // --- queries ------------------------------------------------------------

  snapshotList(): UserTaskSnapshot[] {
    return Array.from(this.tasks.values())
      .map((state) => state.snapshot)
      .sort((a, b) => b.created_at - a.created_at);
  }

  getTaskState(taskId: string): TaskLiveState | undefined {
    return this.tasks.get(taskId);
  }

  // --- commands -----------------------------------------------------------

  /** Refresh the task list from the backend (optionally for one session). */
  async refresh(sessionId?: string): Promise<void> {
    const snapshots = await listTasks(sessionId);
    for (const snapshot of snapshots) {
      this.upsertSnapshot(snapshot);
    }
    this.notifyList();
  }

  /**
   * Submit a task and immediately attach to its stream. Returns the new
   * task id. The stream runs in the background and updates subscribers.
   */
  async submit(sessionId: string, prompt: string): Promise<string> {
    const { task_id } = await submitTask({ session_id: sessionId, prompt });
    // Seed an optimistic snapshot until the first stream event/refresh lands.
    const now = Math.floor(Date.now() / 1000);
    this.upsertSnapshot({
      task_id,
      session_id: sessionId,
      status: 'pending',
      prompt_preview: prompt.slice(0, 200),
      last_event_id: 0,
      error: null,
      created_at: now,
      updated_at: now,
      finished_at: null,
    });
    this.notifyList();
    this.attach(task_id);
    return task_id;
  }

  /** Cancel a task. The terminal event arrives via the live stream. */
  async cancel(taskId: string): Promise<void> {
    await cancelTask(taskId);
  }

  /**
   * Attach to a task's live SSE stream, resuming from the last seen seq. Safe
   * to call repeatedly: a no-op if already attached.
   */
  attach(taskId: string): void {
    if (this.attachControllers.has(taskId)) {
      return;
    }
    const controller = new AbortController();
    this.attachControllers.set(taskId, controller);

    const existing = this.tasks.get(taskId);
    if (existing) {
      this.updateTask(taskId, { ...existing, attached: true });
    }

    const run = async () => {
      try {
        await streamTask(taskId, {
          lastEventId: this.tasks.get(taskId)?.lastSeq,
          signal: controller.signal,
          onEvent: (event) => this.handleEvent(taskId, event),
        });
      } catch {
        // Network/abort error: drop the attach flag so a later call can retry.
      } finally {
        this.attachControllers.delete(taskId);
        const state = this.tasks.get(taskId);
        if (state) {
          this.updateTask(taskId, { ...state, attached: false });
        }
      }
    };
    void run();
  }

  /** Detach a live stream without cancelling the underlying task. */
  detach(taskId: string): void {
    this.attachControllers.get(taskId)?.abort();
    this.attachControllers.delete(taskId);
  }

  // --- event folding ------------------------------------------------------

  private async handleEvent(
    taskId: string,
    event: UserTaskEvent
  ): Promise<void> {
    // The lagged hint frame means we fell behind the server buffer; re-fetch
    // the authoritative snapshot.
    const kind = event.payload?.kind ?? event.kind;
    if (kind === 'lagged') {
      const fresh = await getTask(taskId);
      if (fresh) {
        this.upsertSnapshot(fresh);
        this.notifyList();
      }
      return;
    }

    const state = this.tasks.get(taskId);
    if (!state) {
      // We received an event before any snapshot; fetch one to anchor state.
      const fresh = await getTask(taskId);
      if (fresh) {
        this.upsertSnapshot(fresh);
        this.notifyList();
      }
      return;
    }

    const nextStatus = statusFromEventKind(kind, state.snapshot.status);
    const recentEvents = [...state.recentEvents, event].slice(
      -MAX_RECENT_EVENTS
    );
    const lastSeq =
      event.seq != null ? Math.max(state.lastSeq, event.seq) : state.lastSeq;

    const error =
      event.payload?.kind === 'failed'
        ? event.payload.error
        : state.snapshot.error;

    const updatedSnapshot: UserTaskSnapshot = {
      ...state.snapshot,
      status: nextStatus,
      last_event_id: lastSeq,
      error,
      updated_at: Math.floor(Date.now() / 1000),
      finished_at: isTerminalStatus(nextStatus)
        ? state.snapshot.finished_at ?? Math.floor(Date.now() / 1000)
        : state.snapshot.finished_at,
    };

    this.updateTask(taskId, {
      ...state,
      snapshot: updatedSnapshot,
      recentEvents,
      lastSeq,
    });
    this.notifyList();
  }

  // --- internal state -----------------------------------------------------

  private upsertSnapshot(snapshot: UserTaskSnapshot): void {
    const existing = this.tasks.get(snapshot.task_id);
    if (existing) {
      this.updateTask(snapshot.task_id, {
        ...existing,
        // Preserve a higher locally-tracked seq over a stale list snapshot.
        snapshot: {
          ...snapshot,
          last_event_id: Math.max(
            snapshot.last_event_id,
            existing.snapshot.last_event_id
          ),
        },
      });
    } else {
      this.updateTask(snapshot.task_id, {
        snapshot,
        recentEvents: [],
        lastSeq: snapshot.last_event_id,
        attached: this.attachControllers.has(snapshot.task_id),
      });
    }
  }

  private updateTask(taskId: string, state: TaskLiveState): void {
    this.tasks.set(taskId, state);
    const subs = this.taskSubscribers.get(taskId);
    if (subs) {
      for (const callback of subs) {
        callback(state);
      }
    }
  }

  private notifyList(): void {
    const list = this.snapshotList();
    for (const callback of this.listSubscribers) {
      callback(list);
    }
  }
}

function statusFromEventKind(
  kind: string | undefined,
  current: UserTaskStatus
): UserTaskStatus {
  switch (kind) {
    case 'started':
      return 'running';
    case 'completed':
      return 'completed';
    case 'failed':
      return 'failed';
    case 'cancelled':
      return 'cancelled';
    default:
      // message / control / notification / etc. imply the task is running
      // unless it has already reached a terminal state.
      return isTerminalStatus(current) ? current : 'running';
  }
}

export const taskManager = TaskManager.getInstance();
