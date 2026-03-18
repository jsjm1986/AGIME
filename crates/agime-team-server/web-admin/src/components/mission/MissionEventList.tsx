import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { missionApi, MissionEvent } from '../../api/mission';
import { ApiError } from '../../api/client';
import { formatDateTime } from '../../utils/format';

interface MissionEventListProps {
  missionId: string;
  isLive?: boolean;
  runId?: string | null;
}

type LoadPhase = 'idle' | 'loading' | 'error';
type ViewMode = 'business' | 'debug';
type RunScope = 'current' | 'all';

interface BusinessLogRow {
  key: string;
  dotType: string;
  label: string;
  summary: string;
  createdAt: string;
  rawItems: Array<{
    run_id?: string;
    event_id: number;
    event_type: string;
    payload: Record<string, unknown>;
    created_at: string;
  }>;
}

type ReadableGroup = 'milestone' | 'execution' | 'attention' | 'note';
type ReadableTone = 'neutral' | 'progress' | 'success' | 'warning';

interface ReadableBusinessRow extends BusinessLogRow {
  group: ReadableGroup;
  tone: ReadableTone;
  title: string;
  detail: string;
}

const PAGE_LIMIT = 500;
const MAX_PAGES = 40;
const POLL_INTERVAL_MS = 2500;

function payloadRecord(payload: Record<string, unknown>): Record<string, unknown> {
  if (payload && typeof payload === 'object' && !Array.isArray(payload)) {
    return payload;
  }
  return {};
}

function readString(payload: Record<string, unknown>, key: string): string | null {
  const value = payload[key];
  return typeof value === 'string' && value.trim().length > 0 ? value : null;
}

function readNumber(payload: Record<string, unknown>, key: string): number | null {
  const value = payload[key];
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function clipText(value: string, max = 800): string {
  if (value.length <= max) return value;
  return `${value.slice(0, max)}...`;
}

function parseLooseObject(input: string): Record<string, unknown> | null {
  try {
    const parsed = JSON.parse(input);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
  } catch {
    return null;
  }
  return null;
}

function overlapSuffixPrefix(base: string, next: string): number {
  const max = Math.min(base.length, next.length);
  for (let len = max; len > 0; len -= 1) {
    if (base.slice(base.length - len) === next.slice(0, len)) {
      return len;
    }
  }
  return 0;
}

function normalizeChunks(chunks: string[]): string {
  let merged = '';
  for (const chunk of chunks) {
    const trimmed = chunk.trim();
    if (!trimmed) continue;
    if (!merged) {
      merged = chunk;
      continue;
    }
    if (merged.endsWith(chunk)) {
      // repeated fragment
      continue;
    }
    if (chunk.startsWith(merged)) {
      // cumulative chunk from provider; keep latest full text
      merged = chunk;
      continue;
    }
    const overlap = overlapSuffixPrefix(merged, chunk);
    if (overlap > 0) {
      merged += chunk.slice(overlap);
      continue;
    }
    merged += chunk;
  }
  return merged;
}

function eventDotClass(eventType: string): string {
  switch (eventType) {
    case 'text':
      return 'bg-status-info-text/75';
    case 'thinking':
      return 'bg-primary/75';
    case 'toolcall':
    case 'toolresult':
      return 'bg-status-info-text/75';
    case 'goal_start':
    case 'goal_complete':
    case 'pivot':
    case 'goal_abandoned':
      return 'bg-status-error-text/75';
    case 'done':
      return 'bg-status-success-text/75';
    case 'status':
      return 'bg-status-warning-text/75';
    default:
      return 'bg-muted-foreground/60';
  }
}

function panelToneClass(viewMode: ViewMode): string {
  return viewMode === 'business'
    ? 'border-border/60 bg-[linear-gradient(135deg,rgba(250,247,241,0.78),rgba(255,255,255,0.96))]'
    : 'border-border/60 bg-[linear-gradient(135deg,rgba(246,247,250,0.82),rgba(255,255,255,0.98))]';
}

function eventRunKey(event: MissionEvent): string {
  return event.run_id && event.run_id.trim().length > 0 ? event.run_id : 'legacy';
}

function eventIdentityKey(event: MissionEvent): string {
  return `${eventRunKey(event)}:${event.event_id}`;
}

function mergeEvents(prev: MissionEvent[], incoming: MissionEvent[]): MissionEvent[] {
  if (incoming.length === 0) return prev;
  const byId = new Map<string, MissionEvent>();
  for (const item of prev) byId.set(eventIdentityKey(item), item);
  for (const item of incoming) byId.set(eventIdentityKey(item), item);
  return Array.from(byId.values()).sort((a, b) => {
    const runA = eventRunKey(a);
    const runB = eventRunKey(b);
    if (runA === runB) return a.event_id - b.event_id;
    const timeA = new Date(a.created_at).getTime();
    const timeB = new Date(b.created_at).getTime();
    if (Number.isFinite(timeA) && Number.isFinite(timeB) && timeA !== timeB) {
      return timeA - timeB;
    }
    return runA.localeCompare(runB) || a.event_id - b.event_id;
  });
}

function formatTimestamp(input: string): string {
  const date = new Date(input);
  if (Number.isNaN(date.getTime())) return input;
  return formatDateTime(date);
}

function summarizeDebugEvent(event: MissionEvent, isZh: boolean): string {
  const payload = payloadRecord(event.payload);
  switch (event.event_type) {
    case 'status':
      return readString(payload, 'status') || '';
    case 'text':
    case 'thinking':
      return clipText(readString(payload, 'content') || '');
    case 'toolcall': {
      const name = readString(payload, 'name') || readString(payload, 'id') || 'tool';
      return isZh ? `调用: ${name}` : `Call: ${name}`;
    }
    case 'toolresult': {
      const name = readString(payload, 'name') || readString(payload, 'id') || 'tool';
      const success = payload.success === true;
      const content = readString(payload, 'content');
      if (content) {
        return isZh
          ? `${success ? '成功' : '失败'}: ${name} - ${clipText(content, 300)}`
          : `${success ? 'Success' : 'Failed'}: ${name} - ${clipText(content, 300)}`;
      }
      return isZh ? `${success ? '成功' : '失败'}: ${name}` : `${success ? 'Success' : 'Failed'}: ${name}`;
    }
    case 'workspace_changed':
      return isZh
        ? `工作区已由 ${readString(payload, 'tool_name') || '工具执行'} 更新`
        : `Workspace changed by ${readString(payload, 'tool_name') || 'tool execution'}`;
    case 'turn': {
      const current = readNumber(payload, 'current');
      const max = readNumber(payload, 'max');
      if (current !== null && max !== null) {
        return isZh ? `轮次 ${current}/${max}` : `Turn ${current}/${max}`;
      }
      return isZh ? '轮次进度更新' : 'Turn progress';
    }
    case 'compaction': {
      const strategy = readString(payload, 'strategy') || 'compaction';
      const before = readNumber(payload, 'before_tokens');
      const after = readNumber(payload, 'after_tokens');
      if (before !== null && after !== null) {
        return `${strategy}: ${before} -> ${after} tokens`;
      }
      return strategy;
    }
    case 'session_id':
      return readString(payload, 'session_id') || '';
    case 'done': {
      const status = readString(payload, 'status') || 'done';
      const error = readString(payload, 'error');
      return error ? `${status}: ${error}` : status;
    }
    case 'goal_start': {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const title = readString(payload, 'title') || '';
      return title ? `${goalId}: ${title}` : goalId;
    }
    case 'goal_complete': {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const signal = readString(payload, 'signal') || '';
      return signal ? `${goalId} (${signal})` : goalId;
    }
    case 'pivot': {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const from = readString(payload, 'from_approach') || '';
      const to = readString(payload, 'to_approach') || '';
      if (from || to) return `${goalId}: ${from} -> ${to}`;
      return goalId;
    }
    case 'goal_abandoned': {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const reason = readString(payload, 'reason') || '';
      return reason ? `${goalId}: ${reason}` : goalId;
    }
    default:
      return '';
  }
}

function formatDoneSummary(status: string, error: string | null, isZh: boolean): string {
  if (status === 'completed') return isZh ? '任务执行完成' : 'Mission completed';
  if (status === 'cancelled') return isZh ? '任务已取消' : 'Mission cancelled';
  if (status === 'failed') {
    if (error) return isZh ? `任务失败：${error}` : `Mission failed: ${error}`;
    return isZh ? '任务执行失败' : 'Mission failed';
  }
  if (error) return `${status}: ${error}`;
  return status;
}

function formatStatusSummary(statusRaw: string, isZh: boolean): string {
  const parsed = parseLooseObject(statusRaw);
  if (!parsed) return statusRaw;

  const type = readString(parsed, 'type');
  if (!type) return statusRaw;

  const stepNumber = (() => {
    const stepIndex = readNumber(parsed, 'step_index');
    return stepIndex === null ? null : stepIndex + 1;
  })();

  switch (type) {
    case 'mission_planning':
      return isZh ? '正在规划任务步骤' : 'Planning mission steps';
    case 'mission_planned': {
      const mode = readString(parsed, 'mode');
      if (mode) return isZh ? `任务规划完成（${mode}）` : `Mission planning completed (${mode})`;
      return isZh ? '任务规划完成' : 'Mission planning completed';
    }
    case 'mission_paused': {
      const reason = readString(parsed, 'reason');
      const goalId = readString(parsed, 'goal_id');
      if (goalId) {
        if (reason) return isZh ? `任务已暂停（目标 ${goalId}）：${reason}` : `Mission paused (goal ${goalId}): ${reason}`;
        return isZh ? `任务已暂停（目标 ${goalId}）` : `Mission paused (goal ${goalId})`;
      }
      if (stepNumber !== null) {
        if (reason) return isZh ? `任务已暂停（步骤 ${stepNumber}）：${reason}` : `Mission paused (step ${stepNumber}): ${reason}`;
        return isZh ? `任务已暂停（步骤 ${stepNumber}）` : `Mission paused (step ${stepNumber})`;
      }
      if (reason) return isZh ? `任务已暂停：${reason}` : `Mission paused: ${reason}`;
      return isZh ? '任务已暂停' : 'Mission paused';
    }
    case 'step_start': {
      const title = readString(parsed, 'step_title');
      const total = readNumber(parsed, 'total_steps');
      if (stepNumber !== null && total !== null) {
        if (title) return isZh ? `开始步骤 ${stepNumber}/${total}：${title}` : `Starting step ${stepNumber}/${total}: ${title}`;
        return isZh ? `开始步骤 ${stepNumber}/${total}` : `Starting step ${stepNumber}/${total}`;
      }
      if (stepNumber !== null) return isZh ? `开始步骤 ${stepNumber}` : `Starting step ${stepNumber}`;
      return isZh ? '开始执行步骤' : 'Starting step';
    }
    case 'step_retry': {
      const attempt = readNumber(parsed, 'attempt');
      if (stepNumber !== null && attempt !== null) {
        return isZh ? `步骤 ${stepNumber} 重试（第 ${attempt} 次）` : `Retrying step ${stepNumber} (attempt ${attempt})`;
      }
      return isZh ? '步骤重试中' : 'Retrying step';
    }
    case 'step_validation_failed': {
      const attempt = readNumber(parsed, 'attempt');
      const reason = readString(parsed, 'reason');
      if (stepNumber !== null && attempt !== null && reason) {
        return isZh
          ? `步骤 ${stepNumber} 校验未通过（第 ${attempt} 次）：${reason}`
          : `Step ${stepNumber} validation failed (attempt ${attempt}): ${reason}`;
      }
      if (stepNumber !== null && reason) return isZh ? `步骤 ${stepNumber} 校验未通过：${reason}` : `Step ${stepNumber} validation failed: ${reason}`;
      return isZh ? '步骤校验未通过' : 'Step validation failed';
    }
    case 'step_complete': {
      const tokensUsed = readNumber(parsed, 'tokens_used');
      if (stepNumber !== null && tokensUsed !== null) {
        return isZh ? `步骤 ${stepNumber} 完成（${tokensUsed} tokens）` : `Step ${stepNumber} completed (${tokensUsed} tokens)`;
      }
      if (stepNumber !== null) return isZh ? `步骤 ${stepNumber} 完成` : `Step ${stepNumber} completed`;
      return isZh ? '步骤完成' : 'Step completed';
    }
    case 'tool_task_progress': {
      const toolName = readString(parsed, 'tool_name') || readString(parsed, 'task_id') || 'tool';
      const status = readString(parsed, 'status') || 'working';
      const statusMessage = readString(parsed, 'status_message');
      const pollCount = readNumber(parsed, 'poll_count');
      if (statusMessage && pollCount !== null) {
        return isZh
          ? `工具任务进行中（${toolName} · ${status} · 轮询 ${pollCount}）：${statusMessage}`
          : `Tool task in progress (${toolName} · ${status} · poll ${pollCount}): ${statusMessage}`;
      }
      if (statusMessage) {
        return isZh
          ? `工具任务进行中（${toolName} · ${status}）：${statusMessage}`
          : `Tool task in progress (${toolName} · ${status}): ${statusMessage}`;
      }
      if (pollCount !== null) {
        return isZh
          ? `工具任务进行中（${toolName} · ${status} · 轮询 ${pollCount}）`
          : `Tool task in progress (${toolName} · ${status} · poll ${pollCount})`;
      }
      return isZh ? `工具任务进行中（${toolName} · ${status}）` : `Tool task in progress (${toolName} · ${status})`;
    }
    case 'mission_replanned': {
      const count = readNumber(parsed, 'new_step_count');
      if (count !== null) return isZh ? `任务已重规划（新步骤数 ${count}）` : `Mission replanned (${count} new steps)`;
      return isZh ? '任务已重规划' : 'Mission replanned';
    }
    case 'goal_retry': {
      const goalId = readString(parsed, 'goal_id');
      const attempt = readNumber(parsed, 'attempt');
      if (goalId && attempt !== null) {
        return isZh ? `目标 ${goalId} 重试（第 ${attempt} 次）` : `Retrying goal ${goalId} (attempt ${attempt})`;
      }
      if (goalId) return isZh ? `目标 ${goalId} 重试中` : `Retrying goal ${goalId}`;
      return isZh ? '目标重试中' : 'Retrying goal';
    }
    default:
      return statusRaw;
  }
}

function formatToolResultSummary(
  payload: Record<string, unknown>,
  isZh: boolean,
  fallbackName?: string,
): string {
  const name = readString(payload, 'name') || readString(payload, 'id') || fallbackName || (isZh ? '工具' : 'tool');
  const success = payload.success === true;
  const content = readString(payload, 'content');
  const state = success ? (isZh ? '成功' : 'success') : (isZh ? '失败' : 'failed');
  if (content) {
    return isZh ? `${name} ${state}：${clipText(content, 360)}` : `${name} ${state}: ${clipText(content, 360)}`;
  }
  return isZh ? `${name} ${state}` : `${name} ${state}`;
}

function createBusinessRow(
  dotType: string,
  label: string,
  summary: string,
  groupedEvents: MissionEvent[],
): BusinessLogRow {
  const first = groupedEvents[0];
  const last = groupedEvents[groupedEvents.length - 1];
  const runKey = eventRunKey(first);
  return {
    key: `${runKey}:${first.event_id}-${last.event_id}-${dotType}`,
    dotType,
    label,
    summary,
    createdAt: last.created_at,
    rawItems: groupedEvents.map(event => ({
      run_id: event.run_id,
      event_id: event.event_id,
      event_type: event.event_type,
      payload: payloadRecord(event.payload),
      created_at: event.created_at,
    })),
  };
}

function parseStatusPayload(rawItems: BusinessLogRow['rawItems']): Record<string, unknown> | null {
  const first = rawItems[0];
  if (!first || first.event_type !== 'status') return null;
  const statusRaw = typeof first.payload?.status === 'string' ? first.payload.status : null;
  if (!statusRaw) return null;
  return parseLooseObject(statusRaw);
}

function classifyReadableRow(row: BusinessLogRow, isZh: boolean): ReadableBusinessRow | null {
  const first = row.rawItems[0];
  if (!first) return null;

  const primaryType = first.event_type;
  const statusPayload = parseStatusPayload(row.rawItems);
  const statusType = readString(statusPayload || {}, 'type');
  const summary = row.summary.trim();

  const make = (
    group: ReadableGroup,
    tone: ReadableTone,
    title: string,
    detail: string,
  ): ReadableBusinessRow => ({
    ...row,
    group,
    tone,
    title,
    detail,
  });

  if (primaryType === 'thinking') {
    return make(
      'note',
      'neutral',
      isZh ? '模型在整理思路' : 'Agent is reasoning',
      summary,
    );
  }

  if (primaryType === 'text') {
    return make(
      'note',
      'neutral',
      isZh ? '代理备注' : 'Agent note',
      summary,
    );
  }

  if (primaryType === 'workspace_changed') {
    return make(
      'execution',
      'success',
      isZh ? '已写入工作区' : 'Saved work into the workspace',
      summary,
    );
  }

  if (primaryType === 'toolcall' || primaryType === 'toolresult') {
    const failed = /failed|失败/i.test(summary);
    return make(
      failed ? 'attention' : 'execution',
      failed ? 'warning' : 'success',
      failed
        ? isZh ? '工具执行未成功' : 'Tool execution did not finish cleanly'
        : isZh ? '完成了一次工具执行' : 'Finished a tool action',
      clipText(summary, 280),
    );
  }

  if (primaryType === 'goal_start') {
    return make(
      'milestone',
      'progress',
      isZh ? '开始新的目标' : 'Started a new goal',
      summary,
    );
  }

  if (primaryType === 'goal_complete') {
    return make(
      'milestone',
      'success',
      isZh ? '目标已完成' : 'Goal completed',
      summary,
    );
  }

  if (primaryType === 'pivot') {
    return make(
      'attention',
      'progress',
      isZh ? '系统调整了执行方法' : 'The system changed approach',
      summary,
    );
  }

  if (primaryType === 'goal_abandoned') {
    return make(
      'attention',
      'warning',
      isZh ? '这个子目标没有继续做完' : 'This sub-goal was not finished',
      summary,
    );
  }

  if (primaryType === 'done') {
    const payload = payloadRecord(first.payload);
    const status = readString(payload, 'status') || '';
    if (status === 'completed') {
      return make(
        'milestone',
        'success',
        isZh ? '任务已结束' : 'Mission finished',
        summary,
      );
    }
    return make(
      'attention',
      'warning',
      isZh ? '任务以非正常状态结束' : 'Mission ended with issues',
      summary,
    );
  }

  if (primaryType === 'status') {
    switch (statusType) {
      case 'mission_planning':
        return make(
          'milestone',
          'progress',
          isZh ? '系统正在规划后续工作' : 'Planning the next part of the mission',
          summary,
        );
      case 'mission_planned':
        return make(
          'milestone',
          'success',
          isZh ? '规划已经生成' : 'A plan is ready',
          summary,
        );
      case 'step_start':
        return make(
          'milestone',
          'progress',
          isZh ? '开始一个新的步骤' : 'Started a new step',
          summary,
        );
      case 'step_complete':
        return make(
          'milestone',
          'success',
          isZh ? '一个步骤已完成' : 'A step completed',
          summary,
        );
      case 'step_retry':
      case 'goal_retry':
        return make(
          'attention',
          'progress',
          isZh ? '系统正在换一种方式继续' : 'Trying again with a different pass',
          summary,
        );
      case 'step_validation_failed':
        return make(
          'attention',
          'warning',
          isZh ? '结果还不够完整，系统继续补齐' : 'Output was incomplete and is being repaired',
          summary,
        );
      case 'tool_task_progress':
        return make(
          'execution',
          'progress',
          isZh ? '工具仍在后台执行' : 'A tool task is still running',
          summary,
        );
      case 'mission_replanned':
        return make(
          'attention',
          'progress',
          isZh ? '系统重新调整了计划' : 'The mission was replanned',
          summary,
        );
      case 'mission_paused':
        return make(
          'attention',
          'warning',
          isZh ? '任务已暂停，等待继续' : 'Mission paused and is waiting',
          summary,
        );
      default:
        break;
    }
  }

  if (!summary) return null;

  return make(
    'milestone',
    'neutral',
    row.label,
    clipText(summary, 260),
  );
}

function toneClasses(tone: ReadableTone): string {
  switch (tone) {
    case 'success':
      return 'border-status-success-text/20 bg-status-success-bg text-status-success-text';
    case 'warning':
      return 'border-status-warning-text/20 bg-status-warning-bg text-status-warning-text';
    case 'progress':
      return 'border-status-info-text/20 bg-status-info-bg text-status-info-text';
    default:
      return 'border-border/55 bg-muted/26 text-foreground/78';
  }
}

function timelineAccentClass(tone: ReadableTone): string {
  switch (tone) {
    case 'success':
      return 'bg-status-success-text/80';
    case 'warning':
      return 'bg-status-warning-text/80';
    case 'progress':
      return 'bg-status-info-text/80';
    default:
      return 'bg-muted-foreground/55';
  }
}

function buildBusinessRows(events: MissionEvent[], isZh: boolean): BusinessLogRow[] {
  const rows: BusinessLogRow[] = [];
  let idx = 0;

  while (idx < events.length) {
    const event = events[idx];
    const payload = payloadRecord(event.payload);

    if (event.event_type === 'text' || event.event_type === 'thinking') {
      const kind = event.event_type;
      const grouped: MissionEvent[] = [event];
      const chunks: string[] = [readString(payload, 'content') || ''];
      idx += 1;
      while (idx < events.length && events[idx].event_type === kind) {
        grouped.push(events[idx]);
        chunks.push(readString(payloadRecord(events[idx].payload), 'content') || '');
        idx += 1;
      }
      const merged = normalizeChunks(chunks).trim();
      if (merged) {
        rows.push(
          createBusinessRow(
            kind,
            kind === 'thinking' ? (isZh ? '模型思考' : 'Model thinking') : (isZh ? '模型输出' : 'Model output'),
            clipText(merged, 2200),
            grouped,
          ),
        );
      }
      continue;
    }

    if (event.event_type === 'status') {
      const raw = readString(payload, 'status');
      if (raw) {
        rows.push(
          createBusinessRow('status', isZh ? '状态' : 'Status', formatStatusSummary(raw, isZh), [event]),
        );
      }
      idx += 1;
      continue;
    }

    if (event.event_type === 'toolcall') {
      const toolId = readString(payload, 'id');
      const toolName = readString(payload, 'name') || toolId || (isZh ? '工具' : 'tool');
      const next = events[idx + 1];
      if (next && next.event_type === 'toolresult') {
        const nextPayload = payloadRecord(next.payload);
        const nextId = readString(nextPayload, 'id');
        if (!toolId || !nextId || toolId === nextId) {
          const summary = formatToolResultSummary(nextPayload, isZh, toolName);
          rows.push(
            createBusinessRow(
              'toolresult',
              isZh ? '工具执行' : 'Tool execution',
              summary,
              [event, next],
            ),
          );
          idx += 2;
          continue;
        }
      }
      rows.push(
        createBusinessRow(
          'toolcall',
          isZh ? '工具调用' : 'Tool call',
          isZh ? `调用 ${toolName}` : `Calling ${toolName}`,
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'toolresult') {
      rows.push(
        createBusinessRow(
          'toolresult',
          isZh ? '工具执行' : 'Tool execution',
          formatToolResultSummary(payload, isZh),
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'workspace_changed') {
      rows.push(
        createBusinessRow(
          'status',
          isZh ? '工作区更新' : 'Workspace update',
          isZh
            ? `工作区已由 ${readString(payload, 'tool_name') || '工具执行'} 更新`
            : `Workspace updated by ${readString(payload, 'tool_name') || 'tool execution'}`,
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'goal_start') {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const title = readString(payload, 'title');
      rows.push(
        createBusinessRow(
          'goal_start',
          isZh ? '目标开始' : 'Goal started',
          title ? `${goalId}: ${title}` : goalId,
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'goal_complete') {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const signal = readString(payload, 'signal');
      rows.push(
        createBusinessRow(
          'goal_complete',
          isZh ? '目标完成' : 'Goal completed',
          signal ? `${goalId} (${signal})` : goalId,
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'pivot') {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const from = readString(payload, 'from_approach') || '';
      const to = readString(payload, 'to_approach') || '';
      rows.push(
        createBusinessRow(
          'pivot',
          isZh ? '目标转向' : 'Goal pivot',
          from || to ? `${goalId}: ${from} -> ${to}` : goalId,
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'goal_abandoned') {
      const goalId = readString(payload, 'goal_id') || 'goal';
      const reason = readString(payload, 'reason') || '';
      rows.push(
        createBusinessRow(
          'goal_abandoned',
          isZh ? '目标放弃' : 'Goal abandoned',
          reason ? `${goalId}: ${reason}` : goalId,
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    if (event.event_type === 'done') {
      const status = readString(payload, 'status') || 'done';
      const error = readString(payload, 'error');
      rows.push(
        createBusinessRow(
          'done',
          isZh ? '任务结束' : 'Mission done',
          formatDoneSummary(status, error, isZh),
          [event],
        ),
      );
      idx += 1;
      continue;
    }

    idx += 1;
  }

  if (rows.length > 0) return rows;

  return events
    .map(event => {
      const summary = summarizeDebugEvent(event, isZh);
      if (!summary) return null;
      return createBusinessRow(
        event.event_type,
        isZh ? '运行事件' : 'Runtime event',
        summary,
        [event],
      );
    })
    .filter((row): row is BusinessLogRow => row !== null);
}

export function MissionEventList({ missionId, isLive = false, runId }: MissionEventListProps) {
  const { t, i18n } = useTranslation();
  const isZh = (i18n.resolvedLanguage || i18n.language || '').startsWith('zh');

  const [events, setEvents] = useState<MissionEvent[]>([]);
  const [phase, setPhase] = useState<LoadPhase>('loading');
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('business');
  const [runScope, setRunScope] = useState<RunScope>('current');
  const [expandedRawKeys, setExpandedRawKeys] = useState<Set<string>>(new Set());
  const eventsRef = useRef<MissionEvent[]>([]);
  const syncingRef = useRef(false);

  const currentRunId = (runId || '').trim();
  const runFilter = runScope === 'all'
    ? '__all__'
    : currentRunId.length > 0
    ? currentRunId
    : undefined;

  useEffect(() => {
    eventsRef.current = events;
  }, [events]);

  const fetchEventBatches = useCallback(
    async (afterEventId?: number): Promise<MissionEvent[]> => {
      if (runScope === 'all') {
        // Cross-run mode: always full reload because event_id is per-run sequence.
        return missionApi.listEvents(missionId, undefined, '__all__', PAGE_LIMIT);
      }
      const collected: MissionEvent[] = [];
      let cursor = afterEventId;
      for (let page = 0; page < MAX_PAGES; page++) {
        const batch = await missionApi.listEvents(missionId, cursor, runFilter, PAGE_LIMIT);
        if (!batch || batch.length === 0) break;
        collected.push(...batch);
        cursor = batch[batch.length - 1].event_id;
        if (batch.length < PAGE_LIMIT) break;
      }
      return collected;
    },
    [missionId, runFilter, runScope],
  );

  const syncEvents = useCallback(
    async (fullReload: boolean) => {
      if (syncingRef.current) return;
      syncingRef.current = true;
      const shouldFullReload = fullReload || runScope === 'all';
      if (fullReload) {
        setPhase('loading');
        setError(null);
      }
      try {
        const after = shouldFullReload
          ? undefined
          : eventsRef.current.length > 0
          ? eventsRef.current[eventsRef.current.length - 1].event_id
          : undefined;
        const batch = await fetchEventBatches(after);
        setEvents(prev => (shouldFullReload ? mergeEvents([], batch) : mergeEvents(prev, batch)));
        setPhase('idle');
        setError(null);
      } catch (err) {
        if (err instanceof ApiError && err.status === 404) {
          // Graceful downgrade for older backend builds or missions removed concurrently.
          setEvents(prev => (shouldFullReload ? [] : prev));
          setError(null);
          setPhase('idle');
          return;
        }
        setError(t('mission.runtimeLogsLoadFailed', 'Failed to load runtime logs'));
        setPhase(eventsRef.current.length === 0 ? 'error' : 'idle');
      } finally {
        syncingRef.current = false;
      }
    },
    [fetchEventBatches, runScope, t],
  );

  useEffect(() => {
    setEvents([]);
    eventsRef.current = [];
    setExpandedRawKeys(new Set());
    void syncEvents(true);
  }, [missionId, runFilter, syncEvents]);

  useEffect(() => {
    // Reset run scope when mission selection changes.
    setRunScope('current');
  }, [missionId]);

  useEffect(() => {
    if (!isLive) return;
    const id = window.setInterval(() => {
      void syncEvents(false);
    }, POLL_INTERVAL_MS);
    return () => window.clearInterval(id);
  }, [isLive, syncEvents]);

  const orderedEvents = useMemo(() => events, [events]);
  const businessRows = useMemo(() => buildBusinessRows(orderedEvents, isZh), [orderedEvents, isZh]);
  const readableRows = useMemo(
    () => businessRows.map(row => classifyReadableRow(row, isZh)).filter((row): row is ReadableBusinessRow => row !== null),
    [businessRows, isZh],
  );
  const keyMoments = useMemo(() => readableRows.filter(row => row.group !== 'note'), [readableRows]);
  const noteRows = useMemo(() => readableRows.filter(row => row.group === 'note').slice(-3), [readableRows]);
  const latestState = keyMoments[keyMoments.length - 1] || readableRows[readableRows.length - 1] || null;
  const latestDelivery = useMemo(
    () => [...keyMoments].reverse().find(row => row.group === 'execution' || row.tone === 'success') || null,
    [keyMoments],
  );
  const latestAttention = useMemo(
    () => [...readableRows].reverse().find(row => row.group === 'attention' || row.tone === 'warning') || null,
    [readableRows],
  );
  const displayCount = viewMode === 'business' ? readableRows.length : orderedEvents.length;

  const toggleRaw = (key: string) => {
    setExpandedRawKeys(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  if (phase === 'loading' && orderedEvents.length === 0) {
    return (
      <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
        {t('mission.runtimeLogsLoading', 'Loading runtime logs...')}
      </div>
    );
  }

  if (phase === 'error' && orderedEvents.length === 0) {
    return (
      <div className="h-full flex flex-col items-center justify-center gap-3 text-sm text-muted-foreground">
        <p>{error || t('mission.runtimeLogsLoadFailed', 'Failed to load runtime logs')}</p>
        <button
          onClick={() => void syncEvents(true)}
          className="px-2 py-1 text-xs rounded border border-border hover:bg-accent transition-colors"
        >
          {t('mission.runtimeLogsRetry', 'Retry')}
        </button>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <div className="border-b border-border/50 px-4 py-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[11px] font-medium uppercase tracking-[0.18em] text-muted-foreground/62">
                {t('mission.runtimeLogs', 'Runtime logs')}
              </span>
              <span className="rounded-full border border-border/65 bg-muted/25 px-2 py-0.5 text-[11px] text-muted-foreground">
                {displayCount}
              </span>
              {isLive && (
                <span className="rounded-full border border-[hsl(var(--status-info-text))/0.16] bg-status-info-bg px-2 py-0.5 text-[11px] text-status-info-text">
                  {t('mission.runtimeLogsLive', 'Live updates')}
                </span>
              )}
            </div>
            <p className="mt-2 max-w-3xl text-sm leading-6 text-muted-foreground/78">
              {viewMode === 'business'
                ? t('mission.runtimeLogsBusinessHint', 'Readable view keeps only the meaningful operational story: status shifts, tool work, goal transitions, and generated output. Use debug view when you need raw payloads and lower-level runtime events.')
                : t('mission.runtimeLogsDebugHint', 'Debug view keeps the full runtime trail for investigation. It is intentionally noisy and preserves event payloads for troubleshooting.')}
            </p>
          </div>

          <div className="flex flex-wrap gap-2 text-xs">
            <div className="inline-flex overflow-hidden rounded-full border border-border/80 bg-background/80">
              <button
                onClick={() => setViewMode('business')}
                className={`px-3 py-1.5 transition-colors ${
                  viewMode === 'business' ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/40'
                }`}
              >
                {t('mission.runtimeLogsBusinessView', 'Readable view')}
              </button>
              <button
                onClick={() => setViewMode('debug')}
                className={`border-l border-border/80 px-3 py-1.5 transition-colors ${
                  viewMode === 'debug' ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/40'
                }`}
              >
                {t('mission.runtimeLogsDebugView', 'Debug view')}
              </button>
            </div>

            <div className="inline-flex overflow-hidden rounded-full border border-border/80 bg-background/80">
              <button
                onClick={() => setRunScope('current')}
                className={`px-3 py-1.5 transition-colors ${
                  runScope === 'current' ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/40'
                }`}
              >
                {t('mission.runtimeLogsCurrentRun', 'Current run')}
              </button>
              <button
                onClick={() => setRunScope('all')}
                className={`border-l border-border/80 px-3 py-1.5 transition-colors ${
                  runScope === 'all' ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/40'
                }`}
              >
                {t('mission.runtimeLogsAllRuns', 'All runs')}
              </button>
            </div>

            <button
              onClick={() => void syncEvents(true)}
              className="rounded-full border border-border px-3 py-1.5 transition-colors hover:bg-accent"
            >
              {t('mission.runtimeLogsRefresh', 'Refresh')}
            </button>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-auto px-4 py-4">
        {displayCount === 0 ? (
          <div className="flex h-full items-center justify-center rounded-2xl border border-dashed border-border/60 bg-muted/12 text-sm text-muted-foreground">
            {t('mission.runtimeLogsEmpty', 'No runtime logs yet')}
          </div>
        ) : viewMode === 'business' ? (
          <div className="space-y-4">
            <div className="grid gap-3 xl:grid-cols-[minmax(0,1.35fr)_minmax(0,1fr)_minmax(0,1fr)]">
              <section className={`rounded-[24px] border p-4 shadow-[0_16px_44px_-36px_rgba(47,33,15,0.35)] ${panelToneClass(viewMode)}`}>
                <div className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/68">
                  {t('mission.runtimeLogsCurrentSituation', 'Current situation')}
                </div>
                {latestState ? (
                  <div className="mt-3 flex items-start justify-between gap-3">
                    <div>
                      <h3 className="text-base font-semibold text-foreground">{latestState.title}</h3>
                      <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-foreground/82">
                        {latestState.detail}
                      </p>
                    </div>
                    <span className={`shrink-0 rounded-full border px-2.5 py-1 text-[11px] font-medium ${toneClasses(latestState.tone)}`}>
                      {formatTimestamp(latestState.createdAt)}
                    </span>
                  </div>
                ) : (
                  <p className="mt-3 text-sm text-muted-foreground/78">
                    {t('mission.runtimeLogsNoReadableState', 'There is no readable activity summary yet.')}
                  </p>
                )}
              </section>

              <section className={`rounded-[24px] border p-4 shadow-[0_16px_44px_-36px_rgba(47,33,15,0.35)] ${panelToneClass(viewMode)}`}>
                <div className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/68">
                  {t('mission.runtimeLogsLatestOutput', 'Latest useful output')}
                </div>
                {latestDelivery ? (
                  <>
                    <h3 className="mt-3 text-sm font-semibold text-foreground">{latestDelivery.title}</h3>
                    <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-foreground/78">
                      {latestDelivery.detail}
                    </p>
                    <div className="mt-3 text-xs text-muted-foreground">
                      {formatTimestamp(latestDelivery.createdAt)}
                    </div>
                  </>
                ) : (
                  <p className="mt-3 text-sm text-muted-foreground/78">
                    {t('mission.runtimeLogsNoLatestOutput', 'No clear delivery signal has been captured yet.')}
                  </p>
                )}
              </section>

              <section className={`rounded-[24px] border p-4 shadow-[0_16px_44px_-36px_rgba(47,33,15,0.35)] ${panelToneClass(viewMode)}`}>
                <div className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/68">
                  {t('mission.runtimeLogsAttention', 'Needs attention')}
                </div>
                {latestAttention ? (
                  <>
                    <h3 className="mt-3 text-sm font-semibold text-foreground">{latestAttention.title}</h3>
                    <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-foreground/78">
                      {latestAttention.detail}
                    </p>
                    <div className="mt-3 text-xs text-muted-foreground">
                      {formatTimestamp(latestAttention.createdAt)}
                    </div>
                  </>
                ) : (
                  <p className="mt-3 text-sm text-muted-foreground/78">
                    {t('mission.runtimeLogsNoAttention', 'No blocking signal is exposed right now.')}
                  </p>
                )}
              </section>
            </div>

            <div className="grid gap-4 xl:grid-cols-[minmax(0,1.55fr)_minmax(280px,0.8fr)]">
              <section className={`overflow-hidden rounded-[24px] border shadow-[0_16px_44px_-36px_rgba(47,33,15,0.35)] ${panelToneClass(viewMode)}`}>
                <div className="border-b border-border/35 px-4 py-3">
                  <div className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/68">
                    {t('mission.runtimeLogsKeyMoments', 'Key moments')}
                  </div>
                  <p className="mt-2 text-sm leading-6 text-muted-foreground/78">
                    {t('mission.runtimeLogsKeyMomentsHint', 'This timeline keeps only the events that changed the task state, produced output, or forced the system to change direction.')}
                  </p>
                </div>
                <div className="divide-y divide-border/28">
                  {keyMoments.length === 0 ? (
                    <div className="px-4 py-6 text-sm text-muted-foreground/78">
                      {t('mission.runtimeLogsNoKeyMoments', 'No milestone-style runtime moments have been recorded yet.')}
                    </div>
                  ) : keyMoments.map(row => {
                    const showRaw = expandedRawKeys.has(row.key);
                    return (
                      <div key={row.key} className="px-4 py-4">
                        <div className="flex items-start gap-3">
                          <span className={`mt-1 h-2.5 w-2.5 shrink-0 rounded-full ${timelineAccentClass(row.tone)}`} />
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-2">
                              <h3 className="text-sm font-semibold text-foreground">{row.title}</h3>
                              <span className={`rounded-full border px-2 py-0.5 text-[11px] ${toneClasses(row.tone)}`}>
                                {row.label}
                              </span>
                              <span className="ml-auto text-xs text-muted-foreground">
                                {formatTimestamp(row.createdAt)}
                              </span>
                            </div>
                            <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-foreground/82">
                              {row.detail}
                            </p>
                            <button
                              onClick={() => toggleRaw(row.key)}
                              className="mt-2 text-xs text-muted-foreground hover:text-foreground transition-colors"
                            >
                              {showRaw
                                ? t('mission.runtimeLogsHideRaw', 'Hide raw payload')
                                : t('mission.runtimeLogsViewRaw', 'Inspect raw payload')}
                            </button>
                            {showRaw && (
                              <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-2xl border border-border/55 bg-background/80 p-3 font-mono text-caption">
                                {JSON.stringify(row.rawItems, null, 2)}
                              </pre>
                            )}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </section>

              <section className={`overflow-hidden rounded-[24px] border shadow-[0_16px_44px_-36px_rgba(47,33,15,0.35)] ${panelToneClass(viewMode)}`}>
                <div className="border-b border-border/35 px-4 py-3">
                  <div className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground/68">
                    {t('mission.runtimeLogsAgentNotes', 'Agent notes')}
                  </div>
                  <p className="mt-2 text-sm leading-6 text-muted-foreground/78">
                    {t('mission.runtimeLogsAgentNotesHint', 'These notes explain what the model was considering. They are useful context, but they are not treated as final deliverables.')}
                  </p>
                </div>
                <div className="divide-y divide-border/28">
                  {noteRows.length === 0 ? (
                    <div className="px-4 py-6 text-sm text-muted-foreground/78">
                      {t('mission.runtimeLogsNoAgentNotes', 'No free-form notes were captured in the recent runtime trail.')}
                    </div>
                  ) : noteRows.map(row => {
                    const showRaw = expandedRawKeys.has(row.key);
                    return (
                      <div key={row.key} className="px-4 py-4">
                        <div className="flex items-center gap-2">
                          <span className={`h-2 w-2 rounded-full ${timelineAccentClass(row.tone)}`} />
                          <h3 className="text-sm font-semibold text-foreground">{row.title}</h3>
                          <span className="ml-auto text-xs text-muted-foreground">
                            {formatTimestamp(row.createdAt)}
                          </span>
                        </div>
                        <p className="mt-2 whitespace-pre-wrap break-words text-sm leading-6 text-foreground/76">
                          {row.detail}
                        </p>
                        <button
                          onClick={() => toggleRaw(row.key)}
                          className="mt-2 text-xs text-muted-foreground hover:text-foreground transition-colors"
                        >
                          {showRaw
                            ? t('mission.runtimeLogsHideRaw', 'Hide raw payload')
                            : t('mission.runtimeLogsViewRaw', 'Inspect raw payload')}
                        </button>
                        {showRaw && (
                          <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-2xl border border-border/55 bg-background/80 p-3 font-mono text-caption">
                            {JSON.stringify(row.rawItems, null, 2)}
                          </pre>
                        )}
                      </div>
                    );
                  })}
                </div>
              </section>
            </div>
          </div>
        ) : (
          <div className={`overflow-hidden rounded-2xl border shadow-[0_16px_44px_-36px_rgba(47,33,15,0.35)] ${panelToneClass(viewMode)}`}>
            {orderedEvents.map(event => {
              const summary = summarizeDebugEvent(event, isZh);
              const key = `${eventRunKey(event)}-${event.event_id}-${event.created_at}`;
              const showRaw = expandedRawKeys.has(key);
              return (
                <div key={key} className="border-b border-border/35 px-4 py-3 last:border-b-0">
                  <div className="flex items-center gap-2 text-caption text-muted-foreground">
                    <span className={`h-2 w-2 rounded-full ${eventDotClass(event.event_type)}`} />
                    <span className="uppercase tracking-wide text-muted-foreground/82">{event.event_type}</span>
                    <span className="ml-auto">{formatTimestamp(event.created_at)}</span>
                    <span className="rounded-full border border-border/55 bg-background/72 px-2 py-0.5 tabular-nums">#{event.event_id}</span>
                  </div>
                  <p className="mt-2 whitespace-pre-wrap break-words font-mono text-sm leading-6 text-foreground/84">
                    {summary || t('mission.runtimeLogsUnknown', 'No structured content')}
                  </p>
                  <button
                    onClick={() => toggleRaw(key)}
                    className="mt-2 text-xs text-muted-foreground hover:text-foreground transition-colors"
                  >
                    {showRaw
                      ? t('mission.runtimeLogsHideRaw', 'Hide raw payload')
                      : t('mission.runtimeLogsViewRaw', 'Inspect raw payload')}
                  </button>
                  {showRaw && (
                    <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-words rounded-2xl border border-border/55 bg-background/80 p-3 font-mono text-caption">
                      {JSON.stringify(event.payload, null, 2)}
                    </pre>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>

      {error && orderedEvents.length > 0 && (
        <p className="border-t border-border/50 px-3 py-2 text-xs text-status-warning-text">
          {error}
        </p>
      )}
    </div>
  );
}
