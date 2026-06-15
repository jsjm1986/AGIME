/**
 * scheduledTaskClient - hand-written client for the desktop's scheduled task routes.
 *
 * These routes are intentionally NOT part of the generated OpenAPI client
 * (they are gated behind the `desktop_harness_host` cargo feature). So we call them
 * directly here, following the same pattern as `taskClient.ts`.
 *
 * Auth: the agimed backend authenticates via the `X-Secret-Key` header.
 * Both the base URL and the secret-key header are read from the already-configured
 * generated client, so this works in both the Electron and web renderers.
 */

import { client } from '../api/client.gen';
import type {
  ScheduledTaskSummary,
  ScheduledTaskDetail,
  ScheduledTaskRun,
  ScheduledTaskParseResult,
  CreateTaskRequest,
  CreateFromParseRequest,
  UpdateTaskRequest,
  ParseScheduledTaskRequest,
} from '../types/scheduledTask';

function resolveConfig(): { baseUrl: string; headers: Record<string, string> } {
  const config = client.getConfig();
  const baseUrl = (config.baseUrl ?? '').replace(/\/$/, '');

  const headers: Record<string, string> = {};
  const raw = config.headers as unknown;
  if (
    raw &&
    typeof raw === 'object' &&
    typeof (raw as { forEach?: unknown }).forEach === 'function' &&
    !Array.isArray(raw)
  ) {
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
    // The desktop scheduled-task routes return a JSON body of the shape
    // `{ error: string, hint?: string }` on failure. Surface that instead of
    // a bare status code so callers can show the backend's message + hint.
    let detail = '';
    try {
      const body = await response.text();
      if (body) {
        const parsed = JSON.parse(body) as { error?: string; hint?: string };
        if (parsed.error) {
          detail = parsed.hint ? `${parsed.error} — ${parsed.hint}` : parsed.error;
        } else {
          detail = body;
        }
      }
    } catch {
      // Non-JSON body (or empty) — fall back to the status line below.
    }
    throw new Error(
      detail || `${init.method} ${path} failed: ${response.status}`
    );
  }

  const text = await response.text();
  return (text ? JSON.parse(text) : undefined) as T;
}

/** Parse natural language text into a scheduled task preview. */
export async function parseTaskText(
  text: string,
  timezone?: string
): Promise<ScheduledTaskParseResult> {
  const request: ParseScheduledTaskRequest = { text, timezone };
  const response = await jsonRequest<{ preview: ScheduledTaskParseResult }>(
    '/scheduled-tasks/parse',
    {
      method: 'POST',
      body: JSON.stringify(request),
    }
  );
  return response.preview;
}

/** List all scheduled tasks. */
export async function listTasks(): Promise<ScheduledTaskSummary[]> {
  const response = await jsonRequest<{ tasks: ScheduledTaskSummary[] }>(
    '/scheduled-tasks',
    { method: 'GET' }
  );
  return response.tasks;
}

/** Get a single task with its runs. */
export async function getTask(taskId: string): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}`,
    { method: 'GET' }
  );
  return response.task;
}

/** Create a new scheduled task. */
export async function createTask(
  request: CreateTaskRequest
): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    '/scheduled-tasks',
    {
      method: 'POST',
      body: JSON.stringify(request),
    }
  );
  return response.task;
}

/** Create a task from a parsed preview with optional overrides. */
export async function createFromParse(
  request: CreateFromParseRequest
): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    '/scheduled-tasks/create-from-parse',
    {
      method: 'POST',
      body: JSON.stringify(request),
    }
  );
  return response.task;
}

/** Update an existing scheduled task. */
export async function updateTask(
  taskId: string,
  request: UpdateTaskRequest
): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}`,
    {
      method: 'PATCH',
      body: JSON.stringify(request),
    }
  );
  return response.task;
}

/** Publish a draft task (activate it). */
export async function publishTask(taskId: string): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}/publish`,
    { method: 'POST' }
  );
  return response.task;
}

/** Pause an active task. */
export async function pauseTask(taskId: string): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}/pause`,
    { method: 'POST' }
  );
  return response.task;
}

/** Resume a paused task. */
export async function resumeTask(taskId: string): Promise<ScheduledTaskDetail> {
  const response = await jsonRequest<{ task: ScheduledTaskDetail }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}/resume`,
    { method: 'POST' }
  );
  return response.task;
}

/** Run a task immediately. */
export async function runTaskNow(taskId: string): Promise<ScheduledTaskRun> {
  const response = await jsonRequest<{ run: ScheduledTaskRun }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}/run-now`,
    { method: 'POST' }
  );
  return response.run;
}

/** Delete a scheduled task. */
export async function deleteTask(taskId: string): Promise<void> {
  await jsonRequest<void>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}`,
    { method: 'DELETE' }
  );
}

/** List all runs for a task. */
export async function listTaskRuns(taskId: string): Promise<ScheduledTaskRun[]> {
  const response = await jsonRequest<{ runs: ScheduledTaskRun[] }>(
    `/scheduled-tasks/${encodeURIComponent(taskId)}/runs`,
    { method: 'GET' }
  );
  return response.runs;
}
