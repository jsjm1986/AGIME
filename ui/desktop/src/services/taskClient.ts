/**
 * taskClient - hand-written client for the desktop's user-level long-running
 * task routes (`/tasks`).
 *
 * These routes are intentionally NOT part of the generated OpenAPI client
 * (they are gated behind the `desktop_harness_host` cargo feature, which the
 * schema generator builds without, to avoid CI schema drift). So we call them
 * directly here.
 *
 * Auth: the agimed backend authenticates every non-public route via the
 * `X-Secret-Key` *header* (see crates/agime-server/src/auth.rs). A browser
 * `EventSource` cannot set custom headers, so the SSE attach uses a `fetch`
 * `ReadableStream` reader instead — mirroring how the generated `reply()`
 * client consumes the chat SSE stream. Both the base URL and the secret-key
 * header are read back from the already-configured generated client, so this
 * works in both the Electron and web renderers without re-querying IPC.
 */

import { client } from '../api/client.gen';

export type UserTaskStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'failed'
  | 'cancelled';

export function isTerminalStatus(status: UserTaskStatus): boolean {
  return (
    status === 'completed' || status === 'failed' || status === 'cancelled'
  );
}

/** Mirrors the Rust `UserTaskSnapshot` (serde snake_case). */
export interface UserTaskSnapshot {
  task_id: string;
  session_id: string;
  status: UserTaskStatus;
  prompt_preview: string;
  last_event_id: number;
  error: string | null;
  created_at: number;
  updated_at: number;
  finished_at: number | null;
}

/** Mirrors the Rust `UserTaskEventPayload` (serde tag = "kind", snake_case). */
export type UserTaskEventPayload =
  | { kind: 'started' }
  | { kind: 'message'; message: unknown }
  | { kind: 'conversation_replaced'; conversation: unknown }
  | { kind: 'model_change'; model: string; mode: string }
  | { kind: 'notification'; request_id: string; message: unknown }
  | { kind: 'control'; envelope: unknown }
  | { kind: 'completed' }
  | { kind: 'failed'; error: string }
  | { kind: 'cancelled' }
  // The stream may inject a transport-level hint when the client lagged behind
  // the server's broadcast buffer; re-fetch the snapshot on this.
  | { kind: 'lagged' };

/** Mirrors the Rust `UserTaskEvent`. The `lagged` hint frame carries no seq. */
export interface UserTaskEvent {
  task_id?: string;
  seq?: number;
  payload?: UserTaskEventPayload;
  // The lagged hint is sent as a bare `{ "kind": "lagged" }` object.
  kind?: 'lagged';
}

export interface SubmitTaskRequest {
  session_id: string;
  prompt: string;
}

export interface SubmitTaskResponse {
  task_id: string;
}

function resolveConfig(): { baseUrl: string; headers: Record<string, string> } {
  const config = client.getConfig();
  const baseUrl = (config.baseUrl ?? '').replace(/\/$/, '');

  // The generated client stores headers in whatever shape `setConfig` was
  // given (renderer.tsx passes a plain object). Normalize to a flat record so
  // we can forward the `X-Secret-Key` header on every request.
  const headers: Record<string, string> = {};
  const raw = config.headers as unknown;
  if (
    raw &&
    typeof raw === 'object' &&
    typeof (raw as { forEach?: unknown }).forEach === 'function' &&
    !Array.isArray(raw)
  ) {
    // A `Headers` instance (or any iterable with the same forEach signature).
    (raw as { forEach: (cb: (value: string, key: string) => void) => void }).forEach(
      (value, key) => {
        headers[key] = value;
      }
    );
  } else if (Array.isArray(raw)) {
    for (const entry of raw) {
      if (Array.isArray(entry) && entry.length === 2) {
        headers[String(entry[0])] = String(entry[1]);
      }
    }
  } else if (raw && typeof raw === 'object') {
    for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
      if (value != null) {
        headers[key] = String(value);
      }
    }
  }

  return { baseUrl, headers };
}

async function jsonRequest<T>(
  path: string,
  init: RequestInit & { method: string }
): Promise<T> {
  const { baseUrl, headers } = resolveConfig();
  const response = await fetch(`${baseUrl}${path}`, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...headers,
      ...(init.headers as Record<string, string> | undefined),
    },
  });

  if (!response.ok) {
    throw new Error(`${init.method} ${path} failed: ${response.status}`);
  }

  // Cancel returns an empty body with a status code only.
  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

/** Submit a new long-running task. Resolves to the generated task id. */
export async function submitTask(
  request: SubmitTaskRequest
): Promise<SubmitTaskResponse> {
  return jsonRequest<SubmitTaskResponse>('/tasks', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

/** List tasks, optionally filtered to a session, newest first. */
export async function listTasks(
  sessionId?: string
): Promise<UserTaskSnapshot[]> {
  const query = sessionId
    ? `?session_id=${encodeURIComponent(sessionId)}`
    : '';
  return jsonRequest<UserTaskSnapshot[]>(`/tasks${query}`, { method: 'GET' });
}

/** Fetch a single task snapshot, or `null` if it no longer exists. */
export async function getTask(
  taskId: string
): Promise<UserTaskSnapshot | null> {
  const { baseUrl, headers } = resolveConfig();
  const response = await fetch(
    `${baseUrl}/tasks/${encodeURIComponent(taskId)}`,
    { method: 'GET', headers }
  );
  if (response.status === 404) {
    return null;
  }
  if (!response.ok) {
    throw new Error(`GET /tasks/${taskId} failed: ${response.status}`);
  }
  return (await response.json()) as UserTaskSnapshot;
}

/** Cancel a running task. Idempotent for already-terminal tasks. */
export async function cancelTask(taskId: string): Promise<void> {
  await jsonRequest<void>(`/tasks/${encodeURIComponent(taskId)}/cancel`, {
    method: 'POST',
  });
}

export interface StreamTaskOptions {
  /** Resume after this event seq (events with seq <= this are not replayed). */
  lastEventId?: number;
  /** Abort signal to stop the stream (e.g. on unmount). */
  signal?: AbortSignal;
  /** Called for every parsed event frame, including the `lagged` hint. */
  onEvent: (event: UserTaskEvent) => void;
}

/**
 * Attach to a task's live SSE stream via a `fetch` reader (EventSource cannot
 * send the `X-Secret-Key` header). Resolves when the stream ends (terminal
 * event, server close, or abort). Parses standard SSE framing: `id:` lines set
 * the resumable seq, `data:` lines carry the JSON payload, and a blank line
 * dispatches the accumulated frame.
 */
export async function streamTask(
  taskId: string,
  options: StreamTaskOptions
): Promise<void> {
  const { baseUrl, headers } = resolveConfig();
  const query =
    options.lastEventId != null ? `?last_event_id=${options.lastEventId}` : '';

  const response = await fetch(
    `${baseUrl}/tasks/${encodeURIComponent(taskId)}/stream${query}`,
    {
      method: 'GET',
      headers: { ...headers, Accept: 'text/event-stream' },
      signal: options.signal,
    }
  );

  if (!response.ok || !response.body) {
    throw new Error(`stream /tasks/${taskId} failed: ${response.status}`);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';

  const dispatchFrame = (rawFrame: string) => {
    const dataLines: string[] = [];
    for (const line of rawFrame.split('\n')) {
      // We don't need the `id:` line: the seq is carried inside the JSON
      // payload too, so the caller tracks resume position from the event.
      if (line.startsWith('data:')) {
        dataLines.push(line.slice('data:'.length).trimStart());
      }
    }
    if (dataLines.length === 0) {
      return;
    }
    const json = dataLines.join('\n');
    try {
      options.onEvent(JSON.parse(json) as UserTaskEvent);
    } catch {
      // Ignore malformed frames rather than tearing down the stream.
    }
  };

  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true });

      // SSE frames are separated by a blank line.
      let separator = buffer.indexOf('\n\n');
      while (separator !== -1) {
        const frame = buffer.slice(0, separator);
        buffer = buffer.slice(separator + 2);
        dispatchFrame(frame);
        separator = buffer.indexOf('\n\n');
      }
    }
    // Flush any trailing frame without a terminating blank line.
    if (buffer.trim().length > 0) {
      dispatchFrame(buffer);
    }
  } finally {
    reader.releaseLock();
  }
}
