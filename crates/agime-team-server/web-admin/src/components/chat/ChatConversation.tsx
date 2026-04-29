import { useState, useEffect, useLayoutEffect, useRef, useCallback, useMemo, type ReactNode } from "react";
import {
  Loader2,
  X,
  Bot,
  ChevronDown,
  ChevronRight,
  CheckCircle2,
  ListTodo,
  Sparkles,
  Zap,
  Puzzle,
  Wrench,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import i18n from "../../i18n";
import { useAuth } from "../../contexts/AuthContext";
import {
  chatApi,
  type ChatResponseWarning,
  type ComposerCapabilitiesCatalog,
  type CreateSessionOptions,
  type ChatSessionEvent,
  type ChatWorkspaceFileBlock,
  type DelegationRuntime,
  type DelegationRuntimeEventPayload,
  type SessionTaskItem,
  type SessionTaskSummary,
  type UserChatMemorySuggestion,
} from "../../api/chat";
import { documentApi, type DocumentSummary } from "../../api/documents";
import { ChatMessageBubble } from "./ChatMessageBubble";
import {
  ChatInput,
  type ChatInputComposeRequest,
  type ChatInputQuickActionGroup,
} from "./ChatInput";
import {
  ChatCapabilityPicker,
  type ChatCapabilitySelection,
} from "./ChatCapabilityPicker";
import {
  RELATIONSHIP_MEMORY_UPDATED_EVENT,
  dispatchRelationshipMemoryUpdated,
  type RelationshipMemoryPatchPayload,
  type RelationshipMemoryUpdatedDetail,
} from "./relationshipMemoryEvents";
import { DocumentPicker } from "../documents/DocumentPicker";
import { BottomSheetPanel } from "../mobile/BottomSheetPanel";
import type { TeamAgent } from "../../api/agent";
import type { Message, ToolCallInfo } from "./ChatMessageBubble";

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50MB
const CHAT_DEBUG_VIEW_STORAGE_KEY = "chat:show_tool_debug_messages:v1";
const CAPABILITY_BLOCK_HEADER = bilingual("请优先使用以下能力完成本轮任务：", "Please prioritize using the following capabilities in this turn:");
const MIN_VISIBLE_COMPACTION_FREED_TOKENS = 256;

const AGENT_STATUS_DOT: Record<string, string> = {
  running: "bg-status-success-text",
  error: "bg-status-error-text",
  paused: "bg-status-warning-text",
};

function bilingual(zh: string, en: string): string {
  const lang = i18n.resolvedLanguage || i18n.language || "zh";
  return lang.toLowerCase().startsWith("en") ? en : zh;
}

const FILE_ACCEPT = [
  ".pdf",
  ".doc",
  ".docx",
  ".xls",
  ".xlsx",
  ".ppt",
  ".pptx",
  ".txt",
  ".md",
  ".csv",
  ".json",
  ".xml",
  ".html",
  ".htm",
  ".rtf",
  ".odt",
  ".ods",
  ".odp",
  ".png",
  ".jpg",
  ".jpeg",
  ".gif",
  ".webp",
  ".svg",
].join(",");

function stringArraysEqual(a: string[], b: string[]) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

function normalizeCompactionToken(value: unknown): string {
  return String(value || "")
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, "_");
}

function compactionFreedTokens(detail: {
  before_tokens?: unknown;
  after_tokens?: unknown;
  before?: unknown;
  after?: unknown;
}): number {
  const before = Number(detail.before_tokens ?? detail.before ?? 0);
  const after = Number(detail.after_tokens ?? detail.after ?? 0);
  if (!Number.isFinite(before) || !Number.isFinite(after)) {
    return 0;
  }
  return Math.max(0, before - after);
}

function shouldDisplayCompactionEvent(detail: {
  before_tokens?: unknown;
  after_tokens?: unknown;
  before?: unknown;
  after?: unknown;
  phase?: unknown;
  reason?: unknown;
}): boolean {
  const freed = compactionFreedTokens(detail);
  const phase = normalizeCompactionToken(detail.phase);
  const reason = normalizeCompactionToken(detail.reason);
  const structural =
    phase === "session_memory_compaction" ||
    phase === "committed_collapse" ||
    phase === "staged_collapse" ||
    reason === "session_memory_compaction" ||
    reason === "committed_collapse" ||
    reason === "staged_collapse";
  if (structural) {
    return true;
  }
  if (phase === "projection_refresh" || reason === "projection_refresh") {
    return freed >= MIN_VISIBLE_COMPACTION_FREED_TOKENS;
  }
  return freed >= MIN_VISIBLE_COMPACTION_FREED_TOKENS;
}

function parseCapabilityBlock(text: string): {
  refs: string[];
  remainder: string;
  hasBlock: boolean;
} {
  const normalized = text.replace(/\r\n/g, "\n");
  if (!normalized.startsWith(CAPABILITY_BLOCK_HEADER)) {
    return { refs: [], remainder: text, hasBlock: false };
  }

  const lines = normalized.split("\n");
  let index = 1;
  const refs: string[] = [];

  while (index < lines.length) {
    const match = lines[index].match(
      /^\s*-\s*(\[\[(?:skill|ext):.+?\]\])\s*$/,
    );
    if (!match) {
      break;
    }
    refs.push(match[1]);
    index += 1;
  }

  while (index < lines.length && lines[index].trim() === "") {
    index += 1;
  }

  return {
    refs,
    remainder: lines.slice(index).join("\n"),
    hasBlock: true,
  };
}

function buildCapabilityDraft(refs: string[], remainder: string): string {
  const block = refs.length
    ? `${CAPABILITY_BLOCK_HEADER}\n${refs.map((ref) => `- ${ref}`).join("\n")}`
    : "";
  const body = remainder.trimStart();
  if (block && body) {
    return `${block}\n\n${body}`;
  }
  return block || body;
}

function inferCapabilityNameFromRef(ref: string): string {
  const parts = ref
    .replace(/^\[\[/, "")
    .replace(/\]\]$/, "")
    .split("|");
  return parts[1] || ref;
}

function summarizeMemorySuggestion(
  suggestion: UserChatMemorySuggestion,
): string[] {
  const patch = suggestion.proposed_patch || {};
  const lines: string[] = [];
  if (patch.preferred_address) lines.push(bilingual(`称呼：${patch.preferred_address}`, `Preferred address: ${patch.preferred_address}`));
  if (patch.role_hint) lines.push(bilingual(`角色：${patch.role_hint}`, `Role: ${patch.role_hint}`));
  if (patch.current_focus) lines.push(bilingual(`关注：${patch.current_focus}`, `Focus: ${patch.current_focus}`));
  if (patch.collaboration_preference) {
    lines.push(bilingual(`协作偏好：${patch.collaboration_preference}`, `Collaboration preference: ${patch.collaboration_preference}`));
  }
  if (patch.notes) lines.push(bilingual(`备注：${patch.notes}`, `Notes: ${patch.notes}`));
  return lines;
}

function delegationRuntimeStatusTone(status?: string | null) {
  switch ((status || "").toLowerCase()) {
    case "completed":
      return "bg-status-success-bg text-status-success-text";
    case "failed":
      return "bg-status-error-bg text-status-error-text";
    case "running":
      return "bg-status-info-bg text-status-info-text";
    case "pending":
      return "bg-status-warning-bg text-status-warning-text";
    default:
      return "bg-muted text-muted-foreground";
  }
}

function delegationRuntimeStatusLabel(status?: string | null) {
  switch ((status || "").toLowerCase()) {
    case "completed":
      return bilingual("已完成", "Completed");
    case "failed":
      return bilingual("失败", "Failed");
    case "running":
      return bilingual("运行中", "Running");
    case "pending":
      return bilingual("等待中", "Pending");
    default:
      return bilingual("空闲", "Idle");
  }
}

function delegationWorkerRoleLabel(role?: string | null) {
  switch ((role || "").toLowerCase()) {
    case "leader":
      return bilingual("协调者", "Leader");
    case "subagent":
      return bilingual("独立任务", "Single worker");
    case "swarm_worker":
      return bilingual("并行任务", "Parallel worker");
    case "validation_worker":
      return bilingual("验证任务", "Validation worker");
    default:
      return bilingual("任务", "Worker");
  }
}

function delegationWorkerTitle(worker: DelegationRuntime["workers"][number]) {
  const raw = (worker.title || worker.worker_id || "").trim();
  const index =
    raw.match(/^worker_(\d+)(?:_|$)/i)?.[1] ||
    (worker.worker_id || "").match(/^worker_(\d+)(?:_|$)/i)?.[1];
  if (index) {
    return `Worker ${index}`;
  }
  if ((!raw || raw === worker.worker_id) && worker.role === "subagent") {
    return "Worker";
  }
  return raw || "Worker";
}

function delegationLeaderTitle(title?: string | null) {
  const raw = (title || "").trim();
  if (!raw || raw === "Leader") {
    return bilingual("协调者", "Leader");
  }
  return raw;
}

function buildDelegationRuntimeSummary(runtime: DelegationRuntime | null): string {
  if (!runtime) return bilingual("无委托", "No delegation");
  const running = runtime.workers.filter((worker) => worker.status === "running").length;
  const pending = runtime.workers.filter((worker) => worker.status === "pending").length;
  const completed = runtime.workers.filter(
    (worker) => worker.status === "completed",
  ).length;
  const failed = runtime.workers.filter((worker) => worker.status === "failed").length;
  if (running > 0) {
    return bilingual(
      `${running} worker(s) running${completed > 0 ? `, ${completed} completed` : ""}`,
      `${running} worker(s) running${completed > 0 ? `, ${completed} completed` : ""}`,
    );
  }
  if (pending > 0) {
    return bilingual(`${pending} 个 worker 等待中`, `${pending} worker(s) pending`);
  }
  if (failed > 0) {
    return completed > 0
      ? bilingual(
          `${completed} worker(s) completed, ${failed} failed`,
          `${completed} worker(s) completed, ${failed} failed`,
        )
      : bilingual(`${failed} 个 worker 失败`, `${failed} worker(s) failed`);
  }
  return bilingual(
    `${completed || runtime.workers.length} worker(s) completed`,
    `${completed || runtime.workers.length} worker(s) completed`,
  );
}

function applyDelegationRuntimePatch(
  previous: DelegationRuntime | null,
  payload: DelegationRuntimeEventPayload,
): DelegationRuntime {
  const base: DelegationRuntime = previous
    ? {
        ...previous,
        workers: [...previous.workers],
      }
    : {
        active_run: true,
        mode: payload.mode || "subagent",
        status: payload.status || "running",
        summary: payload.summary || null,
        leader: null,
        workers: [],
      };

  if (payload.mode) {
    base.mode = payload.mode;
  }
  if (payload.status) {
    base.status = payload.status;
  }
  if (typeof payload.summary === "string") {
    base.summary = payload.summary;
  }

  if (payload.worker) {
    const nextWorker = payload.worker;
    const workerKey = nextWorker.worker_id;
    const workerIndex = base.workers.findIndex(
      (worker) => worker.worker_id === workerKey,
    );
    if (workerIndex >= 0) {
      base.workers[workerIndex] = {
        ...base.workers[workerIndex],
        ...nextWorker,
        summary:
          nextWorker.summary ?? base.workers[workerIndex].summary ?? undefined,
        result_summary:
          nextWorker.result_summary ??
          base.workers[workerIndex].result_summary ??
          undefined,
        error: nextWorker.error ?? base.workers[workerIndex].error ?? undefined,
      };
    } else {
      base.workers.push(nextWorker);
    }
  }

  const hasRunning = base.workers.some((worker) => worker.status === "running");
  const hasPending = base.workers.some((worker) => worker.status === "pending");
  const hasFailed = base.workers.some((worker) => worker.status === "failed");
  if (hasRunning) {
    base.status = "running";
  } else if (hasPending) {
    base.status = "pending";
  } else if (hasFailed) {
    base.status = "failed";
  } else {
    base.status = "completed";
  }
  base.active_run = base.status === "running" || base.status === "pending";
  if (!base.summary?.trim()) {
    base.summary = buildDelegationRuntimeSummary(base);
  }
  return base;
}

export interface ChatRuntimeEvent {
  kind:
    | "status"
    | "turn"
    | "toolcall"
    | "toolresult"
    | "compaction"
    | "workspace_changed"
    | "done"
    | "connection"
    | "goal"
    | "text";
  text: string;
  ts: number;
  detail?: Record<string, unknown>;
}

interface ChatConversationProps {
  sessionId: string | null;
  agentId: string;
  agentName: string;
  agent?: TeamAgent | null;
  headerVariant?: "default" | "compact";
  headerLeading?: ReactNode;
  teamId?: string;
  initialAttachedDocIds?: string[];
  /** Optional custom session factory for specialized flows (e.g. portal lab coding sessions) */
  createSession?: (initialMessage: string) => Promise<string>;
  createSessionOptions?: CreateSessionOptions;
  onSessionCreated?: (sessionId: string) => void;
  /** Called when a tool result is received during streaming */
  onToolResult?: (toolName: string, result: string, success: boolean) => void;
  /** Called when processing state changes */
  onProcessingChange?: (processing: boolean) => void;
  /** Called when runtime stream event arrives (for timeline/observability UI) */
  onRuntimeEvent?: (event: ChatRuntimeEvent) => void;
  /** Optional error callback for surfacing failures in parent UI */
  onError?: (message: string) => void;
  /** Optional compose request from parent (prefill or auto-send) */
  composeRequest?: ChatInputComposeRequest | null;
  inputQuickActionGroups?: ChatInputQuickActionGroup[];
  headerActions?: ReactNode;
  composerActions?: ReactNode;
  composerCollapsedActions?: ReactNode;
  beforeSend?: (content: string, sessionId: string | null) => Promise<string> | string;
  enableRelationshipMemory?: boolean;
}

function extractTaggedThinking(source: string): {
  content: string;
  thinking: string;
} {
  if (!source) {
    return { content: "", thinking: "" };
  }

  const lower = source.toLowerCase();
  const contentParts: string[] = [];
  const thinkingParts: string[] = [];
  let cursor = 0;

  while (cursor < source.length) {
    const thinkIndex = lower.indexOf("<think>", cursor);
    const thinkingIndex = lower.indexOf("<thinking>", cursor);
    const candidates = [thinkIndex, thinkingIndex].filter(
      (index) => index >= 0,
    );

    if (candidates.length === 0) {
      contentParts.push(source.slice(cursor));
      break;
    }

    const openIndex = Math.min(...candidates);
    contentParts.push(source.slice(cursor, openIndex));

    const usesLongTag = thinkingIndex >= 0 && thinkingIndex === openIndex;
    const openTag = usesLongTag ? "<thinking>" : "<think>";
    const closeTag = usesLongTag ? "</thinking>" : "</think>";
    const innerStart = openIndex + openTag.length;
    const closeIndex = lower.indexOf(closeTag, innerStart);

    if (closeIndex === -1) {
      thinkingParts.push(source.slice(innerStart));
      break;
    }

    thinkingParts.push(source.slice(innerStart, closeIndex));
    cursor = closeIndex + closeTag.length;
  }

  return {
    content: contentParts.join(""),
    thinking: thinkingParts.join(""),
  };
}

function combineThinkingSegments(
  ...segments: Array<string | null | undefined>
): string | undefined {
  const normalized = segments
    .map((segment) => (segment || "").trim())
    .filter((segment) => segment.length > 0);
  if (normalized.length === 0) {
    return undefined;
  }
  return normalized.join("\n");
}

function deriveAssistantPresentation(
  rawContent?: string,
  rawThinking?: string,
) {
  const extracted = extractTaggedThinking(
    stripAssistantSystemInlineNoise(rawContent || ""),
  );
  return {
    content: extracted.content,
    thinking: combineThinkingSegments(rawThinking, extracted.thinking),
  };
}

function isCompactionInlineMessageText(text: string): boolean {
  const normalized = text.trim().toLowerCase();
  return (
    normalized.startsWith("exceeded auto-compact threshold of ") ||
    normalized.startsWith("context limit reached. compacting to continue conversation...")
  );
}

function stripAssistantSystemInlineNoise(text: string): string {
  let output = text;
  const patterns = [
    /^Exceeded auto-compact threshold of \d+%\.\s*Performing auto-compaction\.\.\.\s*/i,
    /^Context limit reached\.\s*Compacting to continue conversation\.\.\.\s*/i,
  ];
  for (const pattern of patterns) {
    output = output.replace(pattern, "");
  }
  return output;
}

type PersistedToolState = {
  runId?: string | null;
  name?: string;
  result?: string;
  success?: boolean;
  durationMs?: number;
  status?: "running" | "completed" | "failed" | "missing";
};

function stringifyToolResult(value: unknown): string | undefined {
  if (typeof value === "string") {
    return value;
  }
  if (value === null || value === undefined) {
    return undefined;
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function parseEventTimeMs(raw: string | undefined): number | null {
  if (!raw) return null;
  const parsed = Date.parse(raw);
  return Number.isFinite(parsed) ? parsed : null;
}

function buildPersistedToolStateMap(events: ChatSessionEvent[]) {
  const startedAtById = new Map<string, number>();
  const states = new Map<string, PersistedToolState>();

  for (const event of events) {
    const payload =
      event.payload && typeof event.payload === "object"
        ? event.payload
        : ({} as Record<string, unknown>);
    const toolId = String(payload.id || "").trim();
    if (!toolId) {
      continue;
    }

    const toolName =
      typeof payload.name === "string" && payload.name.trim().length > 0
        ? payload.name.trim()
        : undefined;

    if (event.event_type === "toolcall") {
      const eventTs = parseEventTimeMs(event.created_at);
      if (eventTs) {
        startedAtById.set(toolId, eventTs);
      }
      states.set(toolId, {
        ...(states.get(toolId) || {}),
        runId:
          typeof event.run_id === "string" && event.run_id.trim().length > 0
            ? event.run_id.trim()
            : states.get(toolId)?.runId,
        name: toolName || states.get(toolId)?.name,
        status: "running",
      });
      continue;
    }

    if (event.event_type === "toolresult") {
      const durationCandidate = Number(
        payload.duration_ms ?? payload.durationMs ?? 0,
      );
      const eventTs = parseEventTimeMs(event.created_at);
      const startedAt = startedAtById.get(toolId);
      const derivedDuration =
        Number.isFinite(durationCandidate) && durationCandidate > 0
          ? durationCandidate
          : eventTs && startedAt && eventTs >= startedAt
            ? eventTs - startedAt
            : undefined;
      const success = payload.success !== false;
      states.set(toolId, {
        ...(states.get(toolId) || {}),
        runId:
          typeof event.run_id === "string" && event.run_id.trim().length > 0
            ? event.run_id.trim()
            : states.get(toolId)?.runId,
        name: toolName || states.get(toolId)?.name,
        result: stringifyToolResult(payload.content),
        success,
        durationMs: derivedDuration,
        status: success ? "completed" : "failed",
      });
    }
  }

  return states;
}

function attachUnmatchedPersistedToolStatesToLatestAssistant(
  messages: Message[],
  toolStates: Map<string, PersistedToolState>,
  unresolvedStatus: "running" | "missing",
) {
  if (messages.length === 0 || toolStates.size === 0) {
    return messages;
  }

  const latestRunId = Array.from(toolStates.values())
    .map((state) => state.runId?.trim())
    .filter((value): value is string => Boolean(value))
    .slice(-1)[0];

  const referencedToolIds = new Set(
    messages.flatMap((message) => (message.toolCalls || []).map((tool) => tool.id)),
  );

  const syntheticToolCalls = Array.from(toolStates.entries())
    .filter(([id, state]) => {
      if (referencedToolIds.has(id)) {
        return false;
      }
      return !latestRunId || state.runId === latestRunId;
    })
    .map(([id, state]) => ({
      id,
      name: state.name || "tool",
      result: state.result,
      success: state.success,
      durationMs: state.durationMs,
      status: state.status || unresolvedStatus,
    }));

  if (syntheticToolCalls.length === 0) {
    return messages;
  }

  const next = [...messages];
  for (let index = next.length - 1; index >= 0; index -= 1) {
    const message = next[index];
    if (message.role !== "assistant") {
      continue;
    }
    if (message.toolCalls && message.toolCalls.length > 0) {
      return next;
    }
    next[index] = {
      ...message,
      toolCalls: syntheticToolCalls,
    };
    return next;
  }
  return next;
}

function isWorkspaceDeliveryHandoffText(content: string): boolean {
  const normalized = content.trim().toLowerCase();
  if (!normalized) {
    return false;
  }
  return (
    normalized.startsWith("document exported to workspace successfully.") ||
    normalized.startsWith("document access established") ||
    normalized.includes("use developer shell, mcp, or another local tool")
  );
}

function removeRedundantWorkspaceDeliveryHandoffs(messages: Message[]): Message[] {
  const cleaned: Message[] = [];
  let seenAssistantSinceLastUser = false;

  for (const message of messages) {
    if (message.role === "user") {
      seenAssistantSinceLastUser = false;
      cleaned.push(message);
      continue;
    }

    const isRedundantHandoff =
      seenAssistantSinceLastUser &&
      isWorkspaceDeliveryHandoffText(message.content) &&
      !(message.thinking && message.thinking.trim().length > 0) &&
      (message.workspaceFiles?.length || 0) === 0 &&
      (message.toolCalls?.length || 0) === 0;

    if (isRedundantHandoff) {
      continue;
    }

    cleaned.push(message);
    seenAssistantSinceLastUser = true;
  }

  return cleaned;
}

function enrichHistoricalMessagesWithToolStates(
  messages: Message[],
  toolStates: Map<string, PersistedToolState>,
  unresolvedStatus: "running" | "missing",
) {
  const enriched = messages.map((message) => {
    if (!message.toolCalls || message.toolCalls.length === 0) {
      return message;
    }
    return {
      ...message,
      toolCalls: message.toolCalls.map((toolCall) => {
        const persisted = toolStates.get(toolCall.id);
        if (!persisted) {
          return {
            ...toolCall,
            status: toolCall.status || unresolvedStatus,
          };
        }
        return {
          ...toolCall,
          name: persisted.name || toolCall.name,
          result: persisted.result ?? toolCall.result,
          success:
            typeof persisted.success === "boolean"
              ? persisted.success
              : toolCall.success,
          durationMs: persisted.durationMs ?? toolCall.durationMs,
          status: persisted.status || toolCall.status || unresolvedStatus,
        };
      }),
    };
  });
  return removeRedundantWorkspaceDeliveryHandoffs(
    attachUnmatchedPersistedToolStatesToLatestAssistant(
      enriched,
      toolStates,
      unresolvedStatus,
    ),
  );
}

function mergeAssistantTurnMessages(messages: Message[]): Message[] {
  if (messages.length <= 1) {
    return messages;
  }

  const merged: Message[] = [];
  let pendingAssistant: Message | null = null;

  const flushPending = () => {
    if (pendingAssistant) {
      merged.push(pendingAssistant);
      pendingAssistant = null;
    }
  };

  for (const message of messages) {
    if (message.role === "user") {
      flushPending();
      merged.push(message);
      continue;
    }

    if (!pendingAssistant) {
      pendingAssistant = { ...message };
      continue;
    }

    const mergedContentParts: string[] = [
      pendingAssistant.content.trim(),
      message.content.trim(),
    ].filter(Boolean);
    const mergedThinkingParts: string[] = [
      pendingAssistant.thinking?.trim() || "",
      message.thinking?.trim() || "",
    ].filter(Boolean);
    const mergedRawContentParts: string[] = [
      pendingAssistant.rawContent?.trim() || "",
      message.rawContent?.trim() || "",
    ].filter(Boolean);
    const mergedRawThinkingParts: string[] = [
      pendingAssistant.rawThinking?.trim() || "",
      message.rawThinking?.trim() || "",
    ].filter(Boolean);
    const workspaceFilesByPath = new Map<string, ChatWorkspaceFileBlock>();
    for (const file of pendingAssistant.workspaceFiles || []) {
      workspaceFilesByPath.set(file.path, file);
    }
    for (const file of message.workspaceFiles || []) {
      if (!workspaceFilesByPath.has(file.path)) {
        workspaceFilesByPath.set(file.path, file);
      }
    }
    const toolCallsById = new Map<string, ToolCallInfo>();
    for (const toolCall of pendingAssistant.toolCalls || []) {
      toolCallsById.set(toolCall.id, toolCall);
    }
    for (const toolCall of message.toolCalls || []) {
      if (!toolCallsById.has(toolCall.id)) {
        toolCallsById.set(toolCall.id, toolCall);
      }
    }

    pendingAssistant = {
      ...pendingAssistant,
      content: mergedContentParts.join("\n\n"),
      thinking:
        mergedThinkingParts.length > 0
          ? mergedThinkingParts.join("\n\n")
          : undefined,
      rawContent:
        mergedRawContentParts.length > 0
          ? mergedRawContentParts.join("\n\n")
          : undefined,
      rawThinking:
        mergedRawThinkingParts.length > 0
          ? mergedRawThinkingParts.join("\n\n")
          : undefined,
      toolCalls:
        toolCallsById.size > 0 ? Array.from(toolCallsById.values()) : undefined,
      workspaceFiles:
        workspaceFilesByPath.size > 0
          ? Array.from(workspaceFilesByPath.values())
          : undefined,
      turn: message.turn || pendingAssistant.turn,
      compaction: message.compaction || pendingAssistant.compaction,
      isStreaming: pendingAssistant.isStreaming || message.isStreaming,
      timestamp:
        message.timestamp > pendingAssistant.timestamp
          ? message.timestamp
          : pendingAssistant.timestamp,
    };
  }

  flushPending();
  return merged;
}

export function ChatConversation({
  sessionId,
  agentId,
  agentName,
  agent,
  headerVariant = "default",
  headerLeading,
  teamId,
  initialAttachedDocIds,
  createSession,
  createSessionOptions,
  onSessionCreated,
  onToolResult,
  onProcessingChange,
  onRuntimeEvent,
  onError,
  composeRequest,
  inputQuickActionGroups,
  headerActions,
  composerActions,
  composerCollapsedActions,
  beforeSend,
  enableRelationshipMemory = false,
}: ChatConversationProps) {
  const { t } = useTranslation();
  const { user } = useAuth();
  const [messages, setMessages] = useState<Message[]>([]);
  const [isProcessing, setIsProcessing] = useState(false);
  const [liveStatus, setLiveStatus] = useState("");
  const [chatWarnings, setChatWarnings] = useState<ChatResponseWarning[]>([]);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [loading, setLoading] = useState(false);
  const [attachedDocs, setAttachedDocs] = useState<DocumentSummary[]>([]);
  const [showDocPicker, setShowDocPicker] = useState(false);
  const [pendingDocIds, setPendingDocIds] = useState<string[]>(
    initialAttachedDocIds || [],
  );
  const [uploading, setUploading] = useState(false);
  const uploadingRef = useRef(false);
  const [composerFocused, setComposerFocused] = useState(false);
  const [composerToolsOpen, setComposerToolsOpen] = useState(false);
  const [showCapabilityPicker, setShowCapabilityPicker] = useState(false);
  const [capabilityDetailKey, setCapabilityDetailKey] = useState<string | null>(null);
  const [capabilityCatalog, setCapabilityCatalog] =
    useState<ComposerCapabilitiesCatalog | null>(null);
  const [capabilityLoading, setCapabilityLoading] = useState(false);
  const [capabilityError, setCapabilityError] = useState<string | null>(null);
  const [, setDraftContent] = useState("");
  const [selectedCapabilityRefs, setSelectedCapabilityRefs] = useState<string[]>(
    [],
  );
  const [localComposeRequest, setLocalComposeRequest] =
    useState<ChatInputComposeRequest | null>(null);
  const [showCapabilities, setShowCapabilities] = useState(false);
  const [showTasksPanel, setShowTasksPanel] = useState(false);
  const [showDelegationPanel, setShowDelegationPanel] = useState(false);
  const [tasksEnabled, setTasksEnabled] = useState(false);
  const [taskBoardId, setTaskBoardId] = useState<string | null>(null);
  const [currentTasks, setCurrentTasks] = useState<SessionTaskItem[]>([]);
  const [taskSummary, setTaskSummary] = useState<SessionTaskSummary | null>(null);
  const [delegationRuntime, setDelegationRuntime] =
    useState<DelegationRuntime | null>(null);
  const [delegationSupported, setDelegationSupported] = useState(false);
  const [memorySuggestions, setMemorySuggestions] = useState<
    UserChatMemorySuggestion[]
  >([]);
  const [showToolDebugMessages, setShowToolDebugMessages] = useState<boolean>(
    () => {
      try {
        return window.localStorage.getItem(CHAT_DEBUG_VIEW_STORAGE_KEY) === "1";
      } catch {
        return false;
      }
    },
  );
  const fileInputRef = useRef<HTMLInputElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const currentSessionRef = useRef<string | null>(sessionId);
  const justCreatedRef = useRef(false);
  const shouldScrollToBottomOnLoadRef = useRef<boolean>(!!sessionId);
  const optimisticTurnRef = useRef<{
    sessionId: string;
    userMessage: Message;
    assistantMessage: Message;
  } | null>(null);
  const toolCallNamesRef = useRef<Map<string, string>>(new Map());
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef<number | null>(null);
  const processingStartedAtRef = useRef<number | null>(null);
  const lastEventIdRef = useRef<number | null>(null);
  const lastRuntimeEventAtRef = useRef<number>(0);
  const isProcessingRef = useRef(false);
  const sessionSyncInFlightRef = useRef(false);

  useEffect(() => {
    setChatWarnings([]);
  }, [agentId, sessionId]);

  useEffect(() => {
    if (sessionId) {
      return;
    }
    const nextPendingDocIds = initialAttachedDocIds || [];
    setPendingDocIds((prev) =>
      stringArraysEqual(prev, nextPendingDocIds) ? prev : nextPendingDocIds,
    );
  }, [initialAttachedDocIds, sessionId]);

  useEffect(() => {
    if (!teamId || pendingDocIds.length === 0) {
      if (!sessionId) {
        setAttachedDocs((prev) => (prev.length === 0 ? prev : []));
      }
      return;
    }
    const missingIds = pendingDocIds.filter(
      (id) => !attachedDocs.some((doc) => doc.id === id),
    );
    if (missingIds.length === 0) {
      return;
    }
    let cancelled = false;
    documentApi
      .getDocumentsByIds(teamId, missingIds)
      .then((docs) => {
        if (cancelled) {
          return;
        }
        setAttachedDocs((prev) => {
          const existingIds = new Set(prev.map((doc) => doc.id));
          return [...prev, ...docs.filter((doc) => !existingIds.has(doc.id))];
        });
      })
      .catch((error) => {
        if (!cancelled) {
          console.error("Failed to resolve attached documents:", error);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [attachedDocs, pendingDocIds, sessionId, teamId]);

  useEffect(() => {
    if (!showCapabilityPicker || !agentId) {
      return;
    }
    let cancelled = false;
    const loadCapabilities = async () => {
      setCapabilityLoading(true);
      setCapabilityError(null);
      try {
        const catalog = sessionId
          ? await chatApi.getSessionComposerCapabilities(sessionId)
          : await chatApi.getAgentComposerCapabilities(agentId);
        if (!cancelled) {
          setCapabilityCatalog(catalog);
        }
      } catch (error) {
        if (!cancelled) {
          console.error("Failed to load composer capabilities:", error);
          setCapabilityCatalog(null);
            setCapabilityError(
              t(
                "chat.capabilityPickerLoadFailed",
                "Unable to load the callable skill catalog right now. Please try again later.",
              ),
            );
        }
      } finally {
        if (!cancelled) {
          setCapabilityLoading(false);
        }
      }
    };
    loadCapabilities();
    return () => {
      cancelled = true;
    };
  }, [agentId, sessionId, showCapabilityPicker, t]);

  // Keep ref in sync
  useEffect(() => {
    currentSessionRef.current = sessionId;
    lastEventIdRef.current = null;
    if (sessionId) {
      shouldScrollToBottomOnLoadRef.current = true;
    }
  }, [sessionId]);

  const loadRelationshipSuggestions = useCallback(
    async (targetSessionId?: string | null) => {
      if (!enableRelationshipMemory || !teamId || !targetSessionId) {
        setMemorySuggestions([]);
        return;
      }
      try {
        const suggestions = await chatApi.listMemorySuggestions(
          teamId,
          targetSessionId,
        );
        setMemorySuggestions(suggestions);
      } catch (error) {
        console.error("Failed to load chat memory suggestions:", error);
      }
    },
    [enableRelationshipMemory, teamId],
  );

  useEffect(() => {
    if (!enableRelationshipMemory) {
      setMemorySuggestions([]);
      return;
    }
    loadRelationshipSuggestions(sessionId);
  }, [enableRelationshipMemory, loadRelationshipSuggestions, sessionId]);

  useEffect(() => {
    if (!enableRelationshipMemory || !teamId) {
      return;
    }

    const handleRelationshipMemoryUpdated = (rawEvent: Event) => {
      const event = rawEvent as CustomEvent<RelationshipMemoryUpdatedDetail>;
      if (event.detail?.teamId !== teamId) {
        return;
      }

      const currentSessionId = currentSessionRef.current;
      if (
        event.detail?.source === "sidebar" &&
        currentSessionId &&
        event.detail.patch
      ) {
        const patch: RelationshipMemoryPatchPayload = event.detail.patch;
        void chatApi
          .updateMyMemory(teamId, {
            ...patch,
            session_id: currentSessionId,
          })
          .then(() => loadRelationshipSuggestions(currentSessionId))
          .catch((error) => {
            console.error(
              "Failed to sync sidebar relationship memory into current chat session:",
              error,
            );
          });
        return;
      }

      if (currentSessionId) {
        void loadRelationshipSuggestions(currentSessionId);
      }
    };

    window.addEventListener(
      RELATIONSHIP_MEMORY_UPDATED_EVENT,
      handleRelationshipMemoryUpdated as EventListener,
    );

    return () => {
      window.removeEventListener(
        RELATIONSHIP_MEMORY_UPDATED_EVENT,
        handleRelationshipMemoryUpdated as EventListener,
      );
    };
  }, [enableRelationshipMemory, loadRelationshipSuggestions, teamId]);

  const effectiveComposeRequest = localComposeRequest ?? composeRequest ?? null;
  const effectiveComposeCapabilityBlock = useMemo(
    () => parseCapabilityBlock(effectiveComposeRequest?.text || ""),
    [effectiveComposeRequest?.id, effectiveComposeRequest?.text],
  );
  const visibleComposeRequest = useMemo(() => {
    if (!effectiveComposeRequest) {
      return null;
    }
    if (!effectiveComposeCapabilityBlock.hasBlock) {
      return effectiveComposeRequest;
    }
    return {
      ...effectiveComposeRequest,
      text: effectiveComposeCapabilityBlock.remainder,
    };
  }, [effectiveComposeCapabilityBlock, effectiveComposeRequest]);

  const capabilityRefMap = useMemo(() => {
    const entries = new Map<
      string,
      {
        key: string;
        kind: "skill" | "extension";
        name: string;
        displayLineZh: string;
        plainLineZh: string;
        description?: string | null;
        summaryText?: string | null;
        detailText?: string | null;
        detailLang?: string | null;
        detailSource?: string | null;
        badge?: string | null;
      }
    >();

    capabilityCatalog?.skills.forEach((skill) => {
      entries.set(skill.skill_ref, {
        key: `skill:${skill.id}`,
        kind: "skill",
        name: skill.name,
        displayLineZh: skill.display_line_zh,
        plainLineZh: skill.plain_line_zh,
        description: skill.description,
        summaryText: skill.summary_text,
        detailText: skill.detail_text,
        detailLang: skill.detail_lang,
        detailSource: skill.detail_source,
        badge: skill.version ? `v${skill.version}` : null,
      });
    });
    capabilityCatalog?.extensions.forEach((extension) => {
      const badge = extension.type
        ? extension.type === "streamable_http"
          ? "HTTP"
          : extension.type.toUpperCase()
        : extension.class === "builtin"
          ? bilingual("内置", "Built-in")
          : extension.class === "team"
            ? bilingual("团队", "Team")
            : bilingual("扩展", "Extension");
      entries.set(extension.ext_ref, {
        key: `ext:${extension.runtime_name}`,
        kind: "extension",
        name: extension.display_name,
        displayLineZh: extension.display_line_zh,
        plainLineZh: extension.plain_line_zh,
        description: extension.description,
        summaryText: extension.summary_text,
        detailText: extension.detail_text,
        detailLang: extension.detail_lang,
        detailSource: extension.detail_source,
        badge,
      });
    });
    return entries;
  }, [capabilityCatalog]);

  const selectedCapabilityKeys = useMemo(
    () =>
      selectedCapabilityRefs
        .map((ref) => capabilityRefMap.get(ref)?.key)
        .filter((value): value is string => Boolean(value)),
    [capabilityRefMap, selectedCapabilityRefs],
  );

  const selectedCapabilities = useMemo(
    () =>
      selectedCapabilityRefs.map((ref) => {
        const meta = capabilityRefMap.get(ref);
        return {
          ref,
          key: meta?.key ?? ref,
          kind: meta?.kind ?? (ref.startsWith("[[skill:") ? "skill" : "extension"),
          name: meta?.name ?? inferCapabilityNameFromRef(ref),
          displayLineZh: meta?.displayLineZh ?? ref,
          plainLineZh: meta?.plainLineZh ?? inferCapabilityNameFromRef(ref),
          description: meta?.description,
          summaryText: meta?.summaryText,
          detailText: meta?.detailText,
          detailLang: meta?.detailLang,
          detailSource: meta?.detailSource,
          badge: meta?.badge,
        };
      }),
    [capabilityRefMap, selectedCapabilityRefs],
  );

  useEffect(() => {
    if (!effectiveComposeRequest) {
      return;
    }
    const nextRefs = effectiveComposeCapabilityBlock.refs;
    setSelectedCapabilityRefs((prev) =>
      stringArraysEqual(prev, nextRefs) ? prev : nextRefs,
    );
  }, [effectiveComposeCapabilityBlock.refs, effectiveComposeRequest]);

  // Surface processing state to parent and maintain elapsed timer anchors
  useEffect(() => {
    isProcessingRef.current = isProcessing;
    onProcessingChange?.(isProcessing);
    if (isProcessing) {
      if (!processingStartedAtRef.current) {
        processingStartedAtRef.current = Date.now();
      }
    } else {
      processingStartedAtRef.current = null;
      lastRuntimeEventAtRef.current = 0;
      setElapsedSeconds(0);
      reconnectAttemptsRef.current = 0;
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
    }
  }, [isProcessing, onProcessingChange]);

  // Update elapsed seconds while processing
  useEffect(() => {
    if (!isProcessing) return;
    const timer = window.setInterval(() => {
      if (!processingStartedAtRef.current) return;
      setElapsedSeconds(
        Math.max(
          0,
          Math.floor((Date.now() - processingStartedAtRef.current) / 1000),
        ),
      );
    }, 1000);
    return () => window.clearInterval(timer);
  }, [isProcessing]);

  // Load session messages
  useEffect(() => {
    if (!sessionId) {
      optimisticTurnRef.current = null;
      setMessages([]);
      setDelegationRuntime(null);
      setDelegationSupported(false);
      return;
    }
    // Skip loadSession if we just created this session in handleSend
    // (loadSession would overwrite the optimistic messages with empty DB content)
    if (justCreatedRef.current) {
      justCreatedRef.current = false;
      return;
    }
    loadSession(sessionId);
  }, [sessionId]);

  // On first load of a session, wait until layout settles, then jump to the newest message.
  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    if (!shouldScrollToBottomOnLoadRef.current || loading) {
      return;
    }

    let frame1 = 0;
    let frame2 = 0;
    frame1 = window.requestAnimationFrame(() => {
      frame2 = window.requestAnimationFrame(() => {
        shouldScrollToBottomOnLoadRef.current = false;
        messagesEndRef.current?.scrollIntoView({ behavior: "auto", block: "end" });
      });
    });

    return () => {
      window.cancelAnimationFrame(frame1);
      window.cancelAnimationFrame(frame2);
    };
  }, [loading, messages, sessionId]);

  // After initial load, auto-scroll only when user is already near the bottom.
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    if (shouldScrollToBottomOnLoadRef.current) {
      return;
    }
    const threshold = 150;
    const isNearBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight <
      threshold;
    if (isNearBottom) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      optimisticTurnRef.current = null;
      eventSourceRef.current?.close();
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem(
        CHAT_DEBUG_VIEW_STORAGE_KEY,
        showToolDebugMessages ? "1" : "0",
      );
    } catch {
      // Ignore localStorage failures (private mode, etc.)
    }
  }, [showToolDebugMessages]);

  const hydrateHistoricalMessages = async (
    sid: string,
    messagesJson: string,
    isSessionProcessing: boolean,
  ) => {
    const parsed = parseMessages(messagesJson);
    try {
      const events = await chatApi.listSessionEvents(sid, {
        order: "asc",
        limit: 2000,
      });
      return enrichHistoricalMessagesWithToolStates(
        parsed,
        buildPersistedToolStateMap(events),
        isSessionProcessing ? "running" : "missing",
      );
    } catch (error) {
      console.error("Failed to hydrate historical tool events:", error);
      return enrichHistoricalMessagesWithToolStates(
        parsed,
        new Map(),
        isSessionProcessing ? "running" : "missing",
      );
    }
  };

  const mergeHydratedMessagesWithOptimisticTurn = (
    sid: string,
    parsed: Message[],
  ) => {
    const optimistic = optimisticTurnRef.current;
    if (!optimistic || optimistic.sessionId !== sid) {
      return parsed;
    }

    const normalizedOptimisticContent = optimistic.userMessage.content.trim();
    const hasHydratedUserMessage = parsed.some(
      (message) =>
        message.role === "user" &&
        message.content.trim() === normalizedOptimisticContent,
    );

    if (hasHydratedUserMessage) {
      optimisticTurnRef.current = null;
      return parsed;
    }

    const merged: Message[] = [];
    let insertedUserBeforeAssistant = false;

    for (const message of parsed) {
      if (!insertedUserBeforeAssistant && message.role === "assistant") {
        merged.push(optimistic.userMessage);
        insertedUserBeforeAssistant = true;
      }
      merged.push(message);
    }

    if (!insertedUserBeforeAssistant) {
      merged.push(optimistic.userMessage);
    }

    const hasAssistantMessage = merged.some(
      (message) => message.role === "assistant",
    );
    if (!hasAssistantMessage) {
      merged.push(optimistic.assistantMessage);
    }

    return merged;
  };

  const loadSession = async (sid: string) => {
    setLoading(true);
    try {
      const detail = await chatApi.getSession(sid);
      applyTaskDetail(detail);
      applyDelegationDetail(detail);
      let parsed = await hydrateHistoricalMessages(
        sid,
        detail.messages_json,
        detail.is_processing,
      );
      setIsProcessing(detail.is_processing);
      if (detail.is_processing) {
        // Ensure a streaming assistant placeholder exists so SSE events
        // have a target message to append to after component remount.
        const lastMsg = parsed[parsed.length - 1];
        if (!lastMsg || lastMsg.role !== "assistant") {
          parsed.push({
            id: `resume-${Date.now()}`,
            role: "assistant",
            content: "",
            rawContent: "",
            rawThinking: "",
            isStreaming: true,
            timestamp: new Date(),
          });
        } else if (!lastMsg.isStreaming) {
          // DB has a saved assistant message but streaming is still active
          parsed[parsed.length - 1] = { ...lastMsg, isStreaming: true };
        }
        setMessages(mergeHydratedMessagesWithOptimisticTurn(sid, parsed));
        void loadRelationshipSuggestions(sid);
        const resumeLabel = t(
          "chat.resumeProcessing",
          "Session is running, reconnecting stream...",
        );
        setLiveStatus(resumeLabel);
        emitRuntimeEvent("connection", resumeLabel);
        connectStream(sid);
      } else {
        parsed = mergeHydratedMessagesWithOptimisticTurn(sid, parsed);
        setMessages(parsed);
        void loadRelationshipSuggestions(sid);
      }
    } catch (e) {
      console.error("Failed to load session:", e);
    } finally {
      setLoading(false);
    }
  };

  const parseMessages = (json: string): Message[] => {
    const normalizeRole = (rawRole: unknown): Message["role"] | null => {
      if (typeof rawRole === "string") {
        const lowered = rawRole.toLowerCase();
        if (lowered === "user") return "user";
        if (lowered === "assistant") return "assistant";
        return null;
      }
      if (rawRole && typeof rawRole === "object") {
        const obj = rawRole as Record<string, unknown>;
        const roleField = obj.role;
        if (typeof roleField === "string") {
          const lowered = roleField.toLowerCase();
          if (lowered === "user") return "user";
          if (lowered === "assistant") return "assistant";
        }
        if (obj.User !== undefined || obj.user !== undefined) return "user";
        if (obj.Assistant !== undefined || obj.assistant !== undefined)
          return "assistant";
      }
      return null;
    };

    try {
      const raw = JSON.parse(json);
      if (!Array.isArray(raw)) return [];
      const parsed: Message[] = [];
      for (let i = 0; i < raw.length; i += 1) {
        const m = raw[i];
        const role = normalizeRole(m?.role);
        if (!role) continue;
        const meta = m?.metadata || {};
        const userVisible = (meta?.user_visible ?? meta?.userVisible) !== false;
        if (!userVisible) continue;

        let rawText = "";
        let rawThinking = "";
        const toolCalls: Array<{ name: string; id: string }> = [];
        const workspaceFiles: ChatWorkspaceFileBlock[] = [];
        if (typeof m.content === "string") {
          rawText = m.content;
        } else if (Array.isArray(m.content)) {
          for (const c of m.content) {
            const cType = String(c?.type || "").toLowerCase();
            if (cType === "text" || (!cType && c?.text)) {
              rawText += c?.text || "";
              continue;
            }
            if (cType === "thinking" && c?.thinking) {
              rawThinking += c.thinking;
              continue;
            }
            if (
              (cType === "toolrequest" ||
                cType === "tool_request" ||
                cType === "tool_use" ||
                cType === "tooluse") &&
              (c?.toolCall || c?.tool_call || c)
            ) {
              const tc = c.toolCall || c.tool_call || c;
              toolCalls.push({
                id: c.id || tc.id || `hist-tool-${i}-${toolCalls.length}`,
                name: tc.name || "tool",
              });
            }
            if (cType === "systemnotification" && typeof c?.msg === "string") {
              if (isCompactionInlineMessageText(c.msg)) {
                continue;
              }
              rawText += c.msg;
              continue;
            }
            if (
              cType === "workspace_file" &&
              typeof c?.path === "string" &&
              typeof c?.label === "string"
            ) {
              workspaceFiles.push({
                type: "workspace_file",
                path: c.path,
                label: c.label,
                content_type:
                  typeof c?.content_type === "string" ? c.content_type : null,
                size_bytes:
                  typeof c?.size_bytes === "number" ? c.size_bytes : null,
                preview_supported: c?.preview_supported !== false,
              });
            }
          }
        } else if (m.content && typeof m.content === "object") {
          const c = m.content as Record<string, unknown>;
          if (typeof c.text === "string") {
            rawText += c.text;
          } else if (typeof c.msg === "string") {
            rawText += c.msg;
          }
        }

        const visibleAssistant =
          role === "assistant"
            ? deriveAssistantPresentation(rawText, rawThinking)
            : { content: rawText, thinking: undefined as string | undefined };

        const hasContent =
          visibleAssistant.content.trim().length > 0 ||
          (visibleAssistant.thinking || "").trim().length > 0 ||
          toolCalls.length > 0 ||
          workspaceFiles.length > 0;
        if (!hasContent) continue;
        const createdRaw = Number(m?.created ?? m?.timestamp ?? 0);
        const createdMs = Number.isFinite(createdRaw)
          ? createdRaw > 10_000_000_000
            ? createdRaw
            : createdRaw * 1000
          : 0;
        parsed.push({
          id: `hist-${i}`,
          role,
          content: visibleAssistant.content,
          thinking: visibleAssistant.thinking,
          rawContent: role === "assistant" ? rawText : undefined,
          rawThinking: role === "assistant" ? rawThinking : undefined,
          toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
          workspaceFiles:
            role === "assistant" && workspaceFiles.length > 0
              ? workspaceFiles
              : undefined,
          timestamp: createdMs > 0 ? new Date(createdMs) : new Date(),
        });
      }
      return parsed;
    } catch {
      return [];
    }
  };

  const emitRuntimeEvent = useCallback(
    (
      kind: ChatRuntimeEvent["kind"],
      text: string,
      detail?: Record<string, unknown>,
    ) => {
      lastRuntimeEventAtRef.current = Date.now();
      onRuntimeEvent?.({
        kind,
        text,
        ts: Date.now(),
        detail,
      });
    },
    [onRuntimeEvent],
  );

  const formatChatWarning = useCallback(
    (warning: ChatResponseWarning) => {
      if (warning.code === "agent_image_input_unsupported") {
        return t(
          "chat.warnings.agentImageInputUnsupported",
          "This agent is not configured for multimodal input, so images will not be sent directly to the model. Switch to a multimodal agent for direct image understanding, or let this agent use OCR/local tools as a fallback.",
        );
      }
      return warning.message;
    },
    [t],
  );

  const applyChatWarnings = useCallback(
    (warnings?: ChatResponseWarning[]) => {
      if (!warnings?.length) return;
      setChatWarnings(warnings);
      const text = warnings.map(formatChatWarning).join(" ");
      setLiveStatus(text);
      emitRuntimeEvent("status", text, {
        warning_codes: warnings.map((warning) => warning.code),
      });
    },
    [emitRuntimeEvent, formatChatWarning],
  );

  const handleFileUpload = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files;
      if (!files?.length || !teamId || uploadingRef.current) return;
      uploadingRef.current = true;
      setUploading(true);
      try {
        for (const file of Array.from(files)) {
          if (file.size > MAX_FILE_SIZE) {
            onError?.(
              `${file.name}: ${t("documents.fileTooLarge", "File exceeds 50MB limit")}`,
            );
            continue;
          }
          try {
            const doc = await documentApi.uploadDocument(teamId, file);
            setAttachedDocs((prev) =>
              prev.some((d) => d.id === doc.id) ? prev : [...prev, doc],
            );
            setPendingDocIds((prev) =>
              prev.includes(doc.id) ? prev : [...prev, doc.id],
            );
          } catch (err: unknown) {
            const msg = err instanceof Error ? err.message : String(err);
            onError?.(msg || `${file.name} ${t("documents.upload")} failed`);
          }
        }
      } finally {
        uploadingRef.current = false;
        setUploading(false);
        if (fileInputRef.current) fileInputRef.current.value = "";
      }
    },
    [teamId, onError, t],
  );

  const applyCapabilitySelection = useCallback(
    (items: ChatCapabilitySelection[]) => {
      const nextRefs = Array.from(
        new Set([
          ...selectedCapabilityRefs,
          ...items.map((item) => item.ref).filter((value) => value.trim().length > 0),
        ]),
      );
      setSelectedCapabilityRefs(nextRefs);
      setCapabilityDetailKey(null);
      setShowCapabilityPicker(false);
      setComposerToolsOpen(false);
    },
    [selectedCapabilityRefs],
  );

  const removeCapabilityRef = useCallback(
    (ref: string) => {
      setSelectedCapabilityRefs((prev) => prev.filter((item) => item !== ref));
    },
    [],
  );

  const handleSend = useCallback(
    async (content: string) => {
      // M19: Prevent double-click race
      if (isProcessing) return;

      const now = Date.now();
      const selectedRefsSnapshot = [...selectedCapabilityRefs];
      let outgoingContent = content;
      let sid = currentSessionRef.current;
      let sessionWasCreated = false;

      if (beforeSend) {
        try {
          outgoingContent = await beforeSend(content, sid);
        } catch (e) {
          console.error("Failed beforeSend transformation:", e);
          const msg = t("chat.sendFailed", "Request failed");
          setLiveStatus(msg);
          emitRuntimeEvent("done", msg);
          onError?.(msg);
          return;
        }
      }

      // Create session if needed
      if (!sid) {
        try {
          if (createSession) {
            sid = await createSession(content);
          } else {
            const docIds = pendingDocIds.length > 0 ? pendingDocIds : undefined;
            const res = await chatApi.createSession(
              agentId,
              docIds,
              createSessionOptions,
            );
            sid = res.session_id;
            applyChatWarnings(res.warnings);
          }
          currentSessionRef.current = sid;
          lastEventIdRef.current = null;
          setPendingDocIds([]);
          justCreatedRef.current = true;
          sessionWasCreated = true;
        } catch (e) {
          console.error("Failed to create session:", e);
          const msg = t("chat.sessionCreateFailed", "Failed to start session");
          setLiveStatus(msg);
          emitRuntimeEvent("done", msg);
          onError?.(msg);
          return;
        }
      }

      // Attach pending documents to existing session before sending
      if (pendingDocIds.length > 0) {
        try {
          await chatApi.attachDocuments(sid, pendingDocIds);
          setPendingDocIds([]);
          setAttachedDocs([]);
        } catch (e) {
          console.error("Failed to attach documents:", e);
        }
      }

      const finalContent = buildCapabilityDraft(selectedRefsSnapshot, outgoingContent);

      // M16: Use stable IDs for React keys
      const userMsgId = `msg-${now}-user`;
      const assistantMsgId = `msg-${now}-assistant`;
      const optimisticUserMessage: Message = {
        id: userMsgId,
        role: "user",
        content: finalContent,
        timestamp: new Date(),
      };
      const optimisticAssistantMessage: Message = {
        id: assistantMsgId,
        role: "assistant",
        content: "",
        rawContent: "",
        rawThinking: "",
        isStreaming: true,
        timestamp: new Date(),
      };
      optimisticTurnRef.current = {
        sessionId: sid,
        userMessage: optimisticUserMessage,
        assistantMessage: optimisticAssistantMessage,
      };
      // Add user message and placeholder assistant message in a single update
      setMessages((prev) => [
        ...prev,
        optimisticUserMessage,
        optimisticAssistantMessage,
      ]);
      if (sessionWasCreated) {
        onSessionCreated?.(sid);
      }
      setSelectedCapabilityRefs([]);

      setLiveStatus(
        t("chat.requestSent", "Request sent, waiting for agent..."),
      );
      emitRuntimeEvent(
        "status",
        t("chat.requestSent", "Request sent, waiting for agent..."),
      );
      setIsProcessing(true);
      isProcessingRef.current = true;
      processingStartedAtRef.current = Date.now();

      try {
        const response = await chatApi.sendMessage(sid, finalContent);
        applyChatWarnings(response.warnings);
        void loadRelationshipSuggestions(sid);
        connectStream(sid);
      } catch (e) {
        console.error("Failed to send message:", e);
        optimisticTurnRef.current = null;
        setIsProcessing(false);
        setSelectedCapabilityRefs(selectedRefsSnapshot);
        setLocalComposeRequest({
          id: `send-retry:${Date.now()}`,
          text: finalContent,
        });
        const msg = t("chat.sendFailed", "Request failed");
        setLiveStatus(msg);
        emitRuntimeEvent("done", msg);
        onError?.(msg);
        // Remove placeholder
        setMessages((prev) => prev.slice(0, -1));
      }
    },
    [
      agentId,
      applyChatWarnings,
      createSession,
      createSessionOptions,
      emitRuntimeEvent,
      isProcessing,
      loadRelationshipSuggestions,
      beforeSend,
      onSessionCreated,
      onError,
      pendingDocIds,
      selectedCapabilities,
      selectedCapabilityRefs,
      t,
    ],
  );

  const formatStatusLabel = useCallback(
    (raw: string) => {
      try {
        const parsed = JSON.parse(raw || "{}");
        if (parsed?.type === "tool_task_progress") {
          const tool = parsed.tool_name || parsed.task_id || "tool";
          const status = parsed.status || "working";
          const msg = parsed.status_message
            ? ` - ${parsed.status_message}`
            : "";
          return `${tool}: ${status}${msg}`;
        }
      } catch {
        // Non-JSON status; use legacy matching below.
      }
      const status = (raw || "").toLowerCase();
      if (!status) return t("chat.processing", "Processing...");
      if (status === "running") return t("chat.processing", "Processing...");
      if (status.includes("llm"))
        return t("chat.statusLlm", "Calling model...");
      if (status.includes("portal_tool_retry"))
        return t(
          "chat.statusPortalRetry",
          "Portal coding mode: forcing tool execution...",
        );
      if (status.includes("tool"))
        return t("chat.statusTool", "Executing tools...");
      if (status.includes("compaction"))
        return t("chat.statusCompaction", "Compacting context...");
      return raw;
    },
    [t],
  );

  useEffect(() => {
    if (!isProcessing) return;
    if (!lastRuntimeEventAtRef.current) {
      lastRuntimeEventAtRef.current = Date.now();
    }
    const timer = window.setInterval(() => {
      const now = Date.now();
      if (now - lastRuntimeEventAtRef.current < 15000) {
        return;
      }
      const heartbeat = t("chat.statusHeartbeat", "Agent is still running...");
      setLiveStatus(heartbeat);
      emitRuntimeEvent("status", heartbeat, { source: "heartbeat" });
    }, 5000);

    return () => window.clearInterval(timer);
  }, [emitRuntimeEvent, isProcessing, t]);

  const connectStream = (sid: string, isReconnect = false) => {
    // M20: Close any existing EventSource before opening new one
    eventSourceRef.current?.close();
    eventSourceRef.current = null;
    if (!isReconnect) {
      reconnectAttemptsRef.current = 0;
    }

    const es = chatApi.streamChat(sid, lastEventIdRef.current);
    eventSourceRef.current = es;
    const connectedLabel = isReconnect
      ? t("chat.reconnected", "Reconnected, syncing...")
      : t("chat.streamConnected", "Connected, waiting for updates...");
    setLiveStatus(connectedLabel);
    emitRuntimeEvent("connection", connectedLabel);
    es.onopen = () => {
      reconnectAttemptsRef.current = 0;
      const openedLabel = t("chat.processing", "Processing...");
      setLiveStatus(openedLabel);
      emitRuntimeEvent("connection", openedLabel);
    };

    // H6: Wrap all JSON.parse calls in try/catch
    const safeParse = (data: string) => {
      try {
        return JSON.parse(data);
      } catch {
        console.warn("Failed to parse SSE data:", data);
        return null;
      }
    };

    const captureEventId = (evt: Event) => {
      const raw = (evt as MessageEvent).lastEventId;
      const parsed = Number(raw || 0);
      if (Number.isFinite(parsed) && parsed > 0) {
        lastEventIdRef.current = parsed;
      }
    };

    es.addEventListener("text", (e) => {
      // H5: Ignore events for stale sessions
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      if (
        typeof data.content === "string" &&
        data.content.length > 0 &&
        !isCompactionInlineMessageText(data.content)
      ) {
        emitRuntimeEvent("text", data.content, { source: "assistant_stream" });
      }
      updateLastAssistant((msg) => {
        const nextChunk =
          typeof data.content === "string" &&
          !isCompactionInlineMessageText(data.content)
            ? data.content
            : "";
        const nextRawContent =
          (msg.rawContent || "") + nextChunk;
        const derived = deriveAssistantPresentation(
          nextRawContent,
          msg.rawThinking || "",
        );
        return {
          ...msg,
          rawContent: nextRawContent,
          content: derived.content,
          thinking: derived.thinking,
        };
      });
    });

    es.addEventListener("thinking", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      updateLastAssistant((msg) => {
        const nextRawThinking =
          (msg.rawThinking || "") +
          (typeof data.content === "string" ? data.content : "");
        const derived = deriveAssistantPresentation(
          msg.rawContent || "",
          nextRawThinking,
        );
        return {
          ...msg,
          rawThinking: nextRawThinking,
          content: derived.content,
          thinking: derived.thinking,
        };
      });
    });

    es.addEventListener("toolcall", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      if (data.name) {
        const label = t("chat.executingTool", "Executing tool: {{name}}", {
          name: data.name,
        });
        setLiveStatus(label);
        emitRuntimeEvent("toolcall", label, { id: data.id, name: data.name });
      }
      // Track tool call id -> name for onToolResult callback
      if (data.id && data.name) {
        toolCallNamesRef.current.set(data.id, data.name);
      }
      updateLastAssistant((msg) => ({
        ...msg,
        toolCalls: [
          ...(msg.toolCalls || []),
          { name: data.name, id: data.id, status: "running" },
        ],
      }));
    });

    es.addEventListener("toolresult", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const toolName = toolCallNamesRef.current.get(data.id) || data.name || "";
      const durationMs = Number(data.duration_ms ?? data.durationMs ?? 0);
      if (data.id) {
        toolCallNamesRef.current.delete(data.id);
      }
      const resultLabel =
        data.success === false
          ? t("chat.toolFailedBy", "{{name}} failed", {
              name: toolName || t("chat.toolGeneric", "Tool"),
            })
          : t("chat.toolDoneBy", "{{name}} completed", {
              name: toolName || t("chat.toolGeneric", "Tool"),
            });
      const withDuration =
        durationMs > 0
          ? `${resultLabel} (${t("chat.toolDurationMs", "{{n}}ms", { n: durationMs })})`
          : resultLabel;
      setLiveStatus(withDuration);
      emitRuntimeEvent("toolresult", resultLabel, {
        id: data.id,
        success: data.success !== false,
        toolName,
        durationMs,
        preview:
          typeof data.content === "string" ? data.content.slice(0, 200) : "",
      });
      updateLastAssistant((msg) => ({
        ...msg,
        toolCalls: (msg.toolCalls || []).map((tc) =>
          tc.id === data.id
            ? {
                ...tc,
                result: data.content,
                success: data.success,
                durationMs: durationMs > 0 ? durationMs : undefined,
                status: data.success === false ? "failed" : "completed",
              }
            : tc,
        ),
      }));
      // Notify parent about tool results (e.g. for Portal preview refresh)
      if (toolName) {
        onToolResult?.(toolName, data.content || "", data.success !== false);
      }
    });

    es.addEventListener("delegation", (e) => {
      if (currentSessionRef.current !== sid) return;
      const data = safeParse(e.data) as DelegationRuntimeEventPayload | null;
      if (!data) return;
      setDelegationRuntime((prev) => applyDelegationRuntimePatch(prev, data));
    });

    es.addEventListener("turn", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const turnLabel = t("chat.turnProgress", "Turn {{current}}/{{max}}", {
        current: data.current,
        max: data.max,
      });
      setLiveStatus(turnLabel);
      emitRuntimeEvent("turn", turnLabel, {
        current: data.current,
        max: data.max,
      });
      updateLastAssistant((msg) => ({
        ...msg,
        turn: { current: data.current, max: data.max },
      }));
    });

    es.addEventListener("compaction", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      if (!shouldDisplayCompactionEvent(data)) {
        return;
      }
      const compactLabel = t("chat.statusCompaction", "Compacting context...");
      setLiveStatus(compactLabel);
      emitRuntimeEvent("compaction", compactLabel, {
        strategy: data.strategy,
        before: data.before_tokens,
        after: data.after_tokens,
        phase: data.phase,
        reason: data.reason,
      });
      updateLastAssistant((msg) => ({
        ...msg,
        compaction: {
          strategy: data.strategy,
          before: data.before_tokens,
          after: data.after_tokens,
          phase: data.phase,
          reason: data.reason,
        },
      }));
    });

    es.addEventListener("status", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data?.status) return;
      const label = formatStatusLabel(data.status);
      setLiveStatus(label);
      emitRuntimeEvent("status", label, { rawStatus: data.status });
    });

    es.addEventListener("workspace_changed", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      const toolName = data?.tool_name || data?.toolName || "";
      const label = t(
        "chat.workspaceChangedBy",
        "Workspace updated by {{tool}}",
        {
          tool: toolName || "tool",
        },
      );
      setLiveStatus(label);
      emitRuntimeEvent("workspace_changed", label, { toolName });
    });

    es.addEventListener("goal_start", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t("chat.goalStart", "Goal started: {{title}}", {
        title: data.title || data.goal_id || "goal",
      });
      setLiveStatus(label);
      emitRuntimeEvent("goal", label, data);
    });

    es.addEventListener("goal_complete", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t("chat.goalComplete", "Goal completed");
      setLiveStatus(label);
      emitRuntimeEvent("goal", label, data);
    });

    es.addEventListener("pivot", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t("chat.goalPivot", "Plan pivoted");
      setLiveStatus(label);
      emitRuntimeEvent("goal", label, data);
    });

    es.addEventListener("goal_abandoned", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t("chat.goalAbandoned", "Goal abandoned");
      setLiveStatus(label);
      emitRuntimeEvent("goal", label, data);
    });

    es.addEventListener("done", (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      // Show error in the assistant message if execution failed
      if (data?.error) {
        updateLastAssistant((msg) => ({
          ...msg,
          isStreaming: false,
          content: msg.content || `⚠ ${data.error}`,
        }));
      } else {
        updateLastAssistant((msg) => {
          const hasReadableContent =
            msg.content.trim().length > 0 ||
            (msg.thinking || "").trim().length > 0 ||
            (msg.toolCalls?.length || 0) > 0 ||
            (msg.workspaceFiles?.length || 0) > 0;
          return {
            ...msg,
            isStreaming: false,
            content: hasReadableContent
              ? msg.content
              : t(
                  "chat.completedNoText",
                  "Completed. No textual output returned.",
                ),
          };
        });
      }
      setIsProcessing(false);
      isProcessingRef.current = false;
      const doneLabel = data?.error
        ? t("chat.failed", "Execution failed")
        : t("chat.completed", "Completed");
      setLiveStatus(doneLabel);
      emitRuntimeEvent("done", doneLabel, { error: data?.error || null });
      es.close();
      eventSourceRef.current = null;
    });

    es.onerror = () => {
      if (currentSessionRef.current !== sid) return;
      es.close();
      eventSourceRef.current = null;
      if (!isProcessingRef.current) {
        return;
      }
      const nextAttempt = reconnectAttemptsRef.current + 1;
      reconnectAttemptsRef.current = nextAttempt;
      if (nextAttempt > 6) {
        updateLastAssistant((msg) => ({ ...msg, isStreaming: false }));
        setIsProcessing(false);
        isProcessingRef.current = false;
        const disconnectedLabel = t(
          "chat.streamDisconnected",
          "Stream disconnected",
        );
        setLiveStatus(disconnectedLabel);
        emitRuntimeEvent("connection", disconnectedLabel);
        return;
      }

      const reconnectingLabel = t(
        "chat.reconnecting",
        "Connection lost, reconnecting ({{n}})...",
        { n: nextAttempt },
      );
      setLiveStatus(reconnectingLabel);
      emitRuntimeEvent("connection", reconnectingLabel, {
        attempt: nextAttempt,
      });
      const delay = Math.min(1000 * nextAttempt, 5000);
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
      }
      reconnectTimerRef.current = window.setTimeout(async () => {
        try {
          const detail = await chatApi.getSession(sid);
          if (currentSessionRef.current !== sid) return;
          applyTaskDetail(detail);
          applyDelegationDetail(detail);
          if (detail.is_processing) {
            // Sync latest persisted messages before reconnect to reduce visual gaps.
            const parsed = await hydrateHistoricalMessages(
              sid,
              detail.messages_json,
              detail.is_processing,
            );
            if (parsed.length > 0) {
              setMessages(parsed);
            }
            connectStream(sid, true);
          } else {
            // Processing already finished while disconnected.
            // Reload canonical session history to avoid missing final output.
            const parsed = await hydrateHistoricalMessages(
              sid,
              detail.messages_json,
              detail.is_processing,
            );
            if (parsed.length > 0) {
              setMessages(parsed);
            } else {
              updateLastAssistant((msg) => ({ ...msg, isStreaming: false }));
            }
            setIsProcessing(false);
            isProcessingRef.current = false;
            const completedLabel = t("chat.completed", "Completed");
            setLiveStatus(completedLabel);
            emitRuntimeEvent("done", completedLabel);
          }
        } catch {
          if (currentSessionRef.current === sid && isProcessingRef.current) {
            connectStream(sid, true);
          }
        }
      }, delay);
    };
  };

  const updateLastAssistant = (updater: (msg: Message) => Message) => {
    setMessages((prev) => {
      const copy = [...prev];
      for (let i = copy.length - 1; i >= 0; i--) {
        if (copy[i].role === "assistant") {
          copy[i] = updater(copy[i]);
          break;
        }
      }
      return copy;
    });
  };

  const displayMessages = useMemo(() => {
    const turnMergedMessages = mergeAssistantTurnMessages(messages);
    if (showToolDebugMessages) return turnMergedMessages;

    const out: Message[] = [];
    for (const msg of turnMergedMessages) {
      const isToolOnlyAssistant =
        msg.role === "assistant" &&
        (msg.content || "").trim().length === 0 &&
        !(msg.thinking && msg.thinking.trim().length > 0) &&
        (msg.workspaceFiles?.length || 0) === 0 &&
        (msg.toolCalls?.length || 0) > 0;

      if (!isToolOnlyAssistant) {
        out.push(msg);
        continue;
      }

      // In compact mode, merge standalone tool-only bubbles into the nearest
      // previous assistant bubble to avoid noisy "one tool = one bubble".
      let merged = false;
      for (let i = out.length - 1; i >= 0; i -= 1) {
        if (out[i].role !== "assistant") continue;
        out[i] = {
          ...out[i],
          toolCalls: [...(out[i].toolCalls || []), ...(msg.toolCalls || [])],
          turn: msg.turn || out[i].turn,
          compaction: msg.compaction || out[i].compaction,
        };
        merged = true;
        break;
      }

      if (!merged) {
        // No suitable assistant bubble yet; keep one compact synthetic bubble.
        out.push({
          ...msg,
          id: `${msg.id}-compact`,
          content: t("chat.toolRunSummary", "工具执行摘要"),
        });
      }
    }
    return out;
  }, [messages, showToolDebugMessages, t]);

  // Periodic session-state sync fallback:
  // If SSE misses terminal events, recover by reading persisted session state.
  useEffect(() => {
    if (!isProcessing) return;

    const timer = window.setInterval(async () => {
      const sid = currentSessionRef.current;
      if (!sid || !isProcessingRef.current || sessionSyncInFlightRef.current) {
        return;
      }

      sessionSyncInFlightRef.current = true;
      try {
        const detail = await chatApi.getSession(sid);
        if (currentSessionRef.current !== sid) return;
        applyTaskDetail(detail);
        applyDelegationDetail(detail);

        if (!detail.is_processing) {
          const parsed = await hydrateHistoricalMessages(
            sid,
            detail.messages_json,
            detail.is_processing,
          );
          if (parsed.length > 0) {
            setMessages(mergeHydratedMessagesWithOptimisticTurn(sid, parsed));
          } else {
            updateLastAssistant((msg) => ({ ...msg, isStreaming: false }));
          }

          eventSourceRef.current?.close();
          eventSourceRef.current = null;
          setIsProcessing(false);
          isProcessingRef.current = false;
          const completedLabel = t("chat.completed", "Completed");
          setLiveStatus(completedLabel);
          emitRuntimeEvent("done", completedLabel, { source: "session_poll" });
        }
      } catch {
        // Ignore transient polling failures; SSE/retry flow remains primary.
      } finally {
        sessionSyncInFlightRef.current = false;
      }
    }, 5000);

    return () => window.clearInterval(timer);
  }, [emitRuntimeEvent, isProcessing, t]);

  const handleStop = useCallback(async () => {
    const sid = currentSessionRef.current;
    if (!sid) return;
    try {
      await chatApi.cancelChat(sid);
      const cancelledLabel = t("chat.cancelled", "Cancelled");
      setLiveStatus(cancelledLabel);
      setIsProcessing(false);
      isProcessingRef.current = false;
      emitRuntimeEvent("done", cancelledLabel);
    } catch (e) {
      console.error("Failed to cancel:", e);
    }
  }, [emitRuntimeEvent, t]);

  const handleAcceptMemorySuggestion = useCallback(
    async (suggestionId: string) => {
      try {
        await chatApi.acceptMemorySuggestion(suggestionId);
        setMemorySuggestions((prev) =>
          prev.filter((item) => item.suggestion_id !== suggestionId),
        );
        if (teamId) {
          dispatchRelationshipMemoryUpdated({
            teamId,
            source: "chat",
          });
        }
      } catch (error) {
        console.error("Failed to accept memory suggestion:", error);
      }
    },
    [teamId],
  );

  const handleDismissMemorySuggestion = useCallback(async (suggestionId: string) => {
    try {
      await chatApi.dismissMemorySuggestion(suggestionId);
      setMemorySuggestions((prev) =>
        prev.filter((item) => item.suggestion_id !== suggestionId),
      );
    } catch (error) {
      console.error("Failed to dismiss memory suggestion:", error);
    }
  }, []);

  const normalizedAgentName = agentName.trim().toLowerCase();
  const normalizedModelName = (agent?.model || "").trim().toLowerCase();
  const showModelBadge =
    !!agent?.model && normalizedModelName !== normalizedAgentName;
  const hasSecondaryIdentity = !!agent?.description || showModelBadge;
  const compactHeader = headerVariant === "compact";
  const delegationSummary = buildDelegationRuntimeSummary(delegationRuntime);
  const canShowDelegationToggle = delegationSupported || !!delegationRuntime;

  const applyTaskDetail = useCallback(
    (detail: {
      tasks_enabled?: boolean;
      task_board_id?: string | null;
      current_tasks?: SessionTaskItem[];
      task_summary?: SessionTaskSummary | null;
    }) => {
      setTasksEnabled(Boolean(detail.tasks_enabled));
      setTaskBoardId(detail.task_board_id || null);
      setCurrentTasks(detail.current_tasks || []);
      setTaskSummary(detail.task_summary || null);
    },
    [],
  );

  const applyDelegationDetail = useCallback(
    (detail: {
      delegation_runtime?: DelegationRuntime | null;
      harness_capabilities?: {
        subagent_enabled?: boolean;
        swarm_enabled?: boolean;
        worker_peer_messaging_enabled?: boolean;
        auto_swarm_enabled?: boolean;
        validation_worker_enabled?: boolean;
      } | null;
      subagent_enabled?: boolean;
      swarm_enabled?: boolean;
      worker_peer_messaging_enabled?: boolean;
      validation_worker_enabled?: boolean;
    }) => {
      setDelegationRuntime(detail.delegation_runtime || null);
      setDelegationSupported(
        Boolean(
          detail.harness_capabilities?.subagent_enabled ||
            detail.harness_capabilities?.swarm_enabled ||
            detail.harness_capabilities?.worker_peer_messaging_enabled ||
            detail.harness_capabilities?.auto_swarm_enabled ||
            detail.harness_capabilities?.validation_worker_enabled ||
            detail.subagent_enabled ||
            detail.swarm_enabled ||
            detail.worker_peer_messaging_enabled ||
            detail.validation_worker_enabled,
        ),
      );
    },
    [],
  );

  if (loading) {
    return (
      <div className="flex h-full min-h-0 min-w-0 flex-1 items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      {/* Header with agent info */}
      <div className="shrink-0 border-b border-border/60 bg-background">
        <div
          className={`px-3 sm:px-4 ${compactHeader ? "py-2" : "py-2 sm:py-2.5"} flex ${compactHeader ? "items-center gap-2" : "items-start gap-2.5 sm:gap-3"} min-w-0`}
        >
          {headerLeading ? (
            <div className="shrink-0">{headerLeading}</div>
          ) : null}
          <div
            className={`${compactHeader ? "h-8 w-8 rounded-2xl border border-border/60 bg-muted/28" : "h-7 w-7 sm:h-8 sm:w-8 rounded-full bg-muted-foreground/15"} flex items-center justify-center shrink-0`}
          >
            <Bot
              className={`${compactHeader ? "h-3.5 w-3.5" : "h-3.5 w-3.5 sm:h-4 sm:w-4"} text-muted-foreground`}
            />
          </div>
          <div
            className={`flex-1 min-w-0 ${compactHeader ? "" : "space-y-0.5"}`}
          >
            <div
              className={`flex items-center min-w-0 ${compactHeader ? "gap-1.5" : "gap-2"}`}
            >
              <span
                className={`font-medium truncate ${compactHeader ? "text-[12px] leading-4" : "text-[12px] leading-4 sm:text-[13px] sm:leading-5"}`}
              >
                {agentName}
              </span>
              <span
                className={`h-1.5 w-1.5 rounded-full shrink-0 ${
                  AGENT_STATUS_DOT[agent?.status || ""] ||
                  "bg-status-neutral-text"
                }`}
              />
              {showModelBadge && (
                <span
                  className={`hidden sm:inline-flex items-center bg-muted text-muted-foreground rounded px-1.5 py-0.5 shrink-0 ${compactHeader ? "text-[10px]" : "text-caption"}`}
                >
                  {agent.model}
                </span>
              )}
            </div>
            {!compactHeader && hasSecondaryIdentity && (
              <div className="flex items-center gap-1.5 min-h-[18px] min-w-0">
                {showModelBadge && (
                  <span className="sm:hidden inline-flex items-center text-caption bg-muted text-muted-foreground rounded px-1.5 py-0.5 shrink-0">
                    {agent.model}
                  </span>
                )}
                {agent?.description && (
                  <p className="text-caption text-muted-foreground truncate">
                    {agent.description}
                  </p>
                )}
              </div>
            )}
          </div>
          <div
            className={`ml-auto flex items-center shrink-0 ${compactHeader ? "gap-1" : "gap-1.5"}`}
          >
            {headerActions}
            {tasksEnabled && (
              <button
                onClick={() => setShowTasksPanel((value) => !value)}
                className={`${compactHeader ? "h-8 gap-1 rounded-full px-2.5 text-[11px]" : "h-6 gap-1 rounded-md px-1.5 text-[10px] sm:h-7 sm:px-2 sm:text-caption"} inline-flex items-center border border-border/60 text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground`}
              >
                {showTasksPanel ? (
                  <ChevronDown className="h-3.5 w-3.5" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5" />
                )}
                <ListTodo className="h-3.5 w-3.5" />
                {!compactHeader && (
                  <span className="hidden md:inline">
                    {t("chat.tasks", "Tasks")}
                  </span>
                )}
              </button>
            )}
            {canShowDelegationToggle && (
              <button
                onClick={() => setShowDelegationPanel((value) => !value)}
                className={`${compactHeader ? "h-8 gap-1 rounded-full px-2.5 text-[11px]" : "h-6 gap-1 rounded-md px-1.5 text-[10px] sm:h-7 sm:px-2 sm:text-caption"} inline-flex items-center border border-border/60 text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground`}
                title={t("chat.delegation", "协作运行态")}
              >
                {showDelegationPanel ? (
                  <ChevronDown className="h-3.5 w-3.5" />
                ) : (
                  <ChevronRight className="h-3.5 w-3.5" />
                )}
                <Bot className="h-3.5 w-3.5" />
                {!compactHeader && (
                  <span className="hidden md:inline">
                    {t("chat.delegation", "协作运行态")}
                  </span>
                )}
              </button>
            )}
            {agent &&
              (agent.assigned_skills?.length > 0 ||
                agent.enabled_extensions?.length > 0) && (
                <button
                  onClick={() => setShowCapabilities(!showCapabilities)}
                  className={`${compactHeader ? "h-8 gap-1 rounded-full px-2.5 text-[11px]" : "h-6 gap-1 rounded-md px-1.5 text-[10px] sm:h-7 sm:px-2 sm:text-caption"} inline-flex items-center border border-border/60 text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground`}
                >
                  {showCapabilities ? (
                    <ChevronDown className="h-3.5 w-3.5" />
                  ) : (
                    <ChevronRight className="h-3.5 w-3.5" />
                  )}
                  <span className={compactHeader ? "" : "hidden md:inline"}>
                    {compactHeader
                      ? t("chat.capabilitiesShort", "能力")
                      : t("chat.capabilities", "Capabilities")}
                  </span>
                  {!compactHeader && (
                    <span className="md:hidden">
                      {t("chat.capabilitiesShort", "能力")}
                    </span>
                  )}
                </button>
              )}
            <button
              onClick={() => setShowToolDebugMessages((v) => !v)}
              className={`${compactHeader ? "h-8 w-8 justify-center rounded-2xl p-0" : "h-6 gap-1 rounded-md px-1.5 text-[10px] sm:h-7 sm:px-2 sm:text-caption"} inline-flex items-center border transition-colors ${
                showToolDebugMessages
                  ? "text-foreground border-border bg-muted/60"
                  : "text-muted-foreground border-border/50 hover:text-foreground hover:bg-muted/40"
              }`}
              title={
                showToolDebugMessages
                  ? t("chat.switchCompact", "切换为简洁模式")
                  : t("chat.switchDebug", "切换为调试模式")
              }
            >
              <Wrench className="h-3.5 w-3.5" />
              {!compactHeader && (
                <span className="hidden sm:inline">
                  {showToolDebugMessages
                    ? t("chat.debugModeOn", "调试模式")
                    : t("chat.compactModeOn", "简洁模式")}
                </span>
              )}
            </button>
          </div>
        </div>
        {/* Expandable capabilities panel */}
        {showCapabilities && agent && (
          <div
            className={`${compactHeader ? "mx-3 mb-2 mt-2 rounded-[18px] border border-border/60 bg-muted/22 px-3 py-2.5 sm:mx-4" : "flex flex-wrap gap-1.5 bg-muted/30 px-3 pb-2.5 pt-2 sm:px-4 sm:pb-3"}`}
          >
            <div className={compactHeader ? "mb-2 flex items-center justify-between gap-2" : "sr-only"}>
              <span className="text-[11px] font-medium text-muted-foreground">
                {t("chat.capabilities", "Capabilities")}
              </span>
              <span className="text-[10px] text-muted-foreground">
                {t("chat.capabilitiesSummary", "{{skills}} 技能 · {{extensions}} 扩展", {
                  skills:
                    agent.assigned_skills?.filter((s) => s.enabled).length ?? 0,
                  extensions:
                    agent.enabled_extensions?.filter((e) => e.enabled).length ?? 0,
                })}
              </span>
            </div>
            <div className="flex flex-wrap gap-1.5">
            {agent.assigned_skills
              ?.filter((s) => s.enabled)
              .map((skill) => (
                <span
                  key={skill.skill_id}
                  className="inline-flex items-center gap-1 text-caption bg-background border rounded-full px-2 py-0.5"
                >
                  <Zap className="h-3 w-3 text-status-warning-text" />
                  {skill.name}
                </span>
              ))}
            {agent.enabled_extensions
              ?.filter((e) => e.enabled)
              .map((ext) => (
                <span
                  key={ext.extension}
                  className="inline-flex items-center gap-1 text-caption bg-background border rounded-full px-2 py-0.5"
                >
                  <Puzzle className="h-3 w-3 text-status-info-text" />
                  {ext.extension}
                </span>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Live execution status */}
      {isProcessing && (
        <div className="mx-3 mb-1 mt-2 rounded-[14px] border bg-muted/35 px-2.5 py-1.5 text-[11px] text-muted-foreground flex items-center justify-between gap-2 sm:mx-4 sm:mt-3 sm:px-3 sm:py-2 sm:text-xs">
          <span className="truncate">
            {liveStatus || t("chat.processing", "Processing...")}
          </span>
          <span className="shrink-0">
            {t("chat.elapsed", "{{n}}s", { n: elapsedSeconds })}
          </span>
        </div>
      )}

      {chatWarnings.length > 0 && (
        <div className="mx-3 mb-1 mt-2 rounded-[14px] border border-[hsl(var(--status-warning-text))/0.18] bg-status-warning-bg px-3 py-2 text-[12px] text-status-warning-text sm:mx-4">
          {chatWarnings.map((warning) => (
            <div key={`${warning.code}:${warning.message}`}>
              {formatChatWarning(warning)}
            </div>
          ))}
        </div>
      )}

      {/* Messages */}
      <div className="min-h-0 flex-1 overflow-hidden px-3 py-3 sm:p-4">
        <div className="flex h-full min-h-0 gap-3">
          <div
            ref={scrollContainerRef}
            className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden"
          >
            {messages.length === 0 && !isProcessing && (
              <div className="flex items-center justify-center h-full text-muted-foreground text-[13px]">
                {t(
                  "chat.startConversation",
                  "Send a message to start the conversation",
                )}
              </div>
            )}
            {displayMessages.map((msg) => (
              <ChatMessageBubble
                key={msg.id}
                {...msg}
                agentName={agentName}
                userName={user?.display_name}
                sessionId={sessionId || undefined}
                showExecutionDetails={showToolDebugMessages}
              />
            ))}
            <div ref={messagesEndRef} />
          </div>
          {(tasksEnabled && showTasksPanel) || showDelegationPanel ? (
            <aside className="hidden w-80 shrink-0 overflow-y-auto rounded-2xl border border-border/60 bg-muted/18 p-3 lg:block">
              <div className="space-y-3">
                {tasksEnabled && showTasksPanel && (
                  <section className="rounded-xl border border-border/60 bg-background/70 p-3">
                    <div className="flex items-center gap-2">
                      <ListTodo className="h-4 w-4 text-muted-foreground" />
                      <div className="text-sm font-medium text-foreground">
                        {t("chat.tasks", "Tasks")}
                      </div>
                    </div>
                    {taskSummary && (
                      <div className="mt-3 grid grid-cols-3 gap-2 text-center text-[11px]">
                        <div className="rounded-xl bg-background px-2 py-2">
                          <div className="font-semibold text-foreground">
                            {taskSummary.pending_count}
                          </div>
                          <div className="text-muted-foreground">
                            {t("chat.tasksPending", "待办")}
                          </div>
                        </div>
                        <div className="rounded-xl bg-background px-2 py-2">
                          <div className="font-semibold text-foreground">
                            {taskSummary.in_progress_count}
                          </div>
                          <div className="text-muted-foreground">
                            {t("chat.tasksInProgress", "进行中")}
                          </div>
                        </div>
                        <div className="rounded-xl bg-background px-2 py-2">
                          <div className="font-semibold text-foreground">
                            {taskSummary.completed_count}
                          </div>
                          <div className="text-muted-foreground">
                            {t("chat.tasksCompleted", "已完成")}
                          </div>
                        </div>
                      </div>
                    )}
                    {taskBoardId && (
                      <div className="mt-3 text-[11px] text-muted-foreground">
                        {t("chat.taskBoardId", "任务板")}: {taskBoardId}
                      </div>
                    )}
                    <div className="mt-3 space-y-2">
                      {currentTasks.length === 0 ? (
                        <div className="rounded-xl border border-dashed border-border/70 px-3 py-4 text-xs text-muted-foreground">
                          {t("chat.noTasks", "当前没有任务")}
                        </div>
                      ) : (
                        currentTasks.map((task) => {
                          const tone =
                            task.status === "completed"
                              ? "border-status-success-text/30 bg-status-success-text/8"
                              : task.status === "in_progress"
                                ? "border-status-info-text/30 bg-status-info-text/8"
                                : "border-border/60 bg-background";
                          return (
                            <div
                              key={task.id}
                              className={`rounded-xl border px-3 py-2.5 ${tone}`}
                            >
                              <div className="flex items-start justify-between gap-2">
                                <div className="min-w-0">
                                  <div className="text-sm font-medium text-foreground">
                                    {task.subject}
                                  </div>
                                  <div className="mt-1 text-[11px] leading-4 text-muted-foreground">
                                    {task.active_form}
                                  </div>
                                </div>
                                {task.status === "completed" ? (
                                  <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-status-success-text" />
                                ) : (
                                  <span className="mt-0.5 shrink-0 rounded-full border border-border/60 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
                                    {task.status}
                                  </span>
                                )}
                              </div>
                              {task.owner ? (
                                <div className="mt-2 text-[11px] text-muted-foreground">
                                  {t("chat.taskOwner", "负责人")}: {task.owner}
                                </div>
                              ) : null}
                            </div>
                          );
                        })
                      )}
                    </div>
                  </section>
                )}
                {showDelegationPanel && (
                  <section className="rounded-xl border border-border/60 bg-background/70 p-3">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <Bot className="h-4 w-4 text-muted-foreground" />
                        <div className="text-sm font-medium text-foreground">
                          {t("chat.delegation", "协作运行态")}
                        </div>
                      </div>
                      <span
                        className={`rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                          delegationRuntime?.status,
                        )}`}
                      >
                        {delegationRuntimeStatusLabel(delegationRuntime?.status)}
                      </span>
                    </div>
                    <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                      {delegationSummary}
                    </div>
                    {delegationRuntime?.leader ? (
                      <div className="mt-3 rounded-xl border border-border/60 bg-background px-3 py-2.5">
                        <div className="flex items-center justify-between gap-2">
                          <div>
                            <div className="text-[11px] uppercase tracking-wide text-muted-foreground">
                              Leader
                            </div>
                            <div className="text-sm font-medium text-foreground">
                              {delegationLeaderTitle(delegationRuntime.leader.title)}
                            </div>
                          </div>
                          <span
                            className={`rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                              delegationRuntime.leader.status,
                            )}`}
                          >
                            {delegationRuntimeStatusLabel(delegationRuntime.leader.status)}
                          </span>
                        </div>
                        {delegationRuntime.leader.summary ? (
                          <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                            {delegationRuntime.leader.summary}
                          </div>
                        ) : null}
                      </div>
                    ) : null}
                    <div className="mt-3 space-y-2">
                      {delegationRuntime?.workers?.length ? (
                        delegationRuntime.workers.map((worker) => (
                          <div
                            key={worker.worker_id}
                            className="rounded-xl border border-border/60 bg-background px-3 py-2.5"
                          >
                            <div className="flex items-start justify-between gap-2">
                              <div className="min-w-0">
                                <div className="text-sm font-medium text-foreground">
                                  {delegationWorkerTitle(worker)}
                                </div>
                                <div className="mt-1 text-[11px] uppercase tracking-wide text-muted-foreground">
                                  {delegationWorkerRoleLabel(worker.role)}
                                </div>
                              </div>
                              <span
                                className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                                  worker.status,
                                )}`}
                              >
                                {delegationRuntimeStatusLabel(worker.status)}
                              </span>
                            </div>
                            {worker.summary ? (
                              <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                                {worker.summary}
                              </div>
                            ) : null}
                            {worker.result_summary ? (
                              <div className="mt-2 text-[11px] leading-5 text-foreground">
                                {worker.result_summary}
                              </div>
                            ) : null}
                            {worker.error ? (
                              <div className="mt-2 text-[11px] leading-5 text-status-error-text">
                                {worker.error}
                              </div>
                            ) : null}
                          </div>
                        ))
                      ) : (
                        <div className="rounded-xl border border-dashed border-border/70 px-3 py-4 text-xs text-muted-foreground">
                          {t(
                            "chat.noDelegationRuntime",
                            "No subagent or swarm runtime has happened yet.",
                          )}
                        </div>
                      )}
                    </div>
                  </section>
                )}
              </div>
            </aside>
          ) : null}
        </div>
      </div>

      {/* Attached documents chips */}
      {(attachedDocs.length > 0 || pendingDocIds.length > 0) && (
        <div className="shrink-0 flex flex-wrap items-center gap-1 px-3 pt-1.5 sm:px-4 sm:pt-2">
          {attachedDocs.map((doc) => (
            <span
              key={doc.id}
              className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-1 text-[11px]"
            >
              {doc.display_name || doc.name}
              <button
                onClick={() => {
                  setAttachedDocs((prev) =>
                    prev.filter((d) => d.id !== doc.id),
                  );
                  setPendingDocIds((prev) =>
                    prev.filter((id) => id !== doc.id),
                  );
                }}
              >
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      {selectedCapabilities.length > 0 && (
        <div className="shrink-0 px-3 pt-1.5 sm:px-4 sm:pt-2">
          <div className="rounded-[20px] border border-primary/15 bg-primary/[0.045] px-3 py-2.5 shadow-[inset_0_1px_0_rgba(255,255,255,0.3)]">
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0">
                <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.14em] text-primary/72">
                  <Sparkles className="h-3.5 w-3.5" />
                  <span>{t("chat.selectedCapabilitiesCard", "已选能力")}</span>
                </div>
                <div className="mt-1 text-[12px] text-foreground">
                  {t(
                    "chat.selectedCapabilitiesInlineHint",
                    "{{count}} capability references are attached to this turn and will be sent automatically.",
                    { count: selectedCapabilities.length },
                  )}
                </div>
              </div>
              <button
                type="button"
                onClick={() => {
                  setCapabilityDetailKey(null);
                  setShowCapabilityPicker(true);
                }}
                className="shrink-0 rounded-full border border-primary/20 bg-background/88 px-3 py-1 text-[11px] font-medium text-primary transition-colors hover:bg-background"
              >
                {t("chat.manageCapabilities", "管理")}
              </button>
            </div>

            <div className="-mx-1 mt-2 flex gap-2 overflow-x-auto px-1 pb-1">
              {selectedCapabilities.map((item) => (
                <div
                  key={item.key}
                  className="group relative min-w-[144px] max-w-[200px] shrink-0 rounded-[16px] border border-primary/16 bg-background/92 px-3 py-2 shadow-sm"
                >
                  <button
                    type="button"
                    onClick={() => {
                      setCapabilityDetailKey(item.key);
                      setShowCapabilityPicker(true);
                    }}
                    className="block w-full pr-6 text-left"
                    title={t("chat.capabilityPickerViewDetail", "查看解读")}
                  >
                    <div className="flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-[0.12em] text-muted-foreground">
                      {item.kind === "skill" ? (
                        <Zap className="h-3.5 w-3.5 text-primary" />
                      ) : (
                        <Puzzle className="h-3.5 w-3.5 text-primary" />
                      )}
                      <span>
                        {item.kind === "skill"
                          ? t("chat.capabilityKindSkill", "技能")
                          : t("chat.capabilityKindExtension", "MCP / 扩展")}
                      </span>
                    </div>
                    <div className="mt-1 truncate text-[13px] font-semibold text-foreground">
                      {item.name}
                    </div>
                    <div className="mt-1 line-clamp-2 text-[11px] leading-4 text-muted-foreground">
                      {item.summaryText ||
                        item.plainLineZh ||
                        t(
                          "chat.capabilityPickerNoDetail",
                          "No extra capability explanation is available right now. You can insert it directly into the composer.",
                        )}
                    </div>
                  </button>
                  <button
                    type="button"
                    onClick={() => removeCapabilityRef(item.ref)}
                    className="absolute right-2 top-2 inline-flex h-5 w-5 items-center justify-center rounded-full border border-border/60 bg-background/92 text-[12px] text-muted-foreground transition-colors hover:border-primary/30 hover:text-primary"
                    title={t("chat.removeCapability", "移除该能力引用")}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {enableRelationshipMemory && memorySuggestions.length > 0 && (
        <div className="shrink-0 px-3 pt-1.5 sm:px-4 sm:pt-2">
          <div className="rounded-[18px] border border-amber-200/80 bg-amber-50/70 px-3 py-2.5 text-[12px] text-amber-950 shadow-sm">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-amber-700">
                  {t("chat.relationshipMemory.suggestionTitle", "个人记忆建议")}
                </div>
                <div className="mt-1 text-[13px] font-medium text-foreground">
                  {memorySuggestions[0].reason}
                </div>
                <div className="mt-1 space-y-1 text-[12px] leading-5 text-muted-foreground">
                  {summarizeMemorySuggestion(memorySuggestions[0]).map((line) => (
                    <div key={line}>{line}</div>
                  ))}
                  <div>{t("chat.relationshipMemory.scopeHint", "只作用于你在当前团队下的普通对话。")}</div>
                </div>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() =>
                    handleDismissMemorySuggestion(
                      memorySuggestions[0].suggestion_id,
                    )
                  }
                  className="inline-flex h-8 items-center rounded-full border border-border/70 bg-background px-3 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-muted/40"
                >
                  {t("chat.relationshipMemory.dismiss", "忽略")}
                </button>
                <button
                  type="button"
                  onClick={() =>
                    handleAcceptMemorySuggestion(
                      memorySuggestions[0].suggestion_id,
                    )
                  }
                  className="inline-flex h-8 items-center rounded-full bg-foreground px-3 text-[12px] font-medium text-background transition-colors hover:opacity-90"
                >
                  {t("chat.relationshipMemory.accept", "记住")}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Input with attach button */}
      <div className="mt-auto flex min-w-0 shrink-0 items-end gap-1 border-t border-border/50 bg-background/96 backdrop-blur-sm">
        {(teamId || agentId) && (
          <div className="mb-2 flex items-center gap-1 pl-2 sm:mb-4 sm:pl-2">
            {composerFocused ? (
              <button
                type="button"
                onClick={() => setComposerToolsOpen(true)}
                className="inline-flex h-9 items-center gap-1 rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45 sm:h-10 sm:text-[12px]"
              >
                <span>{t("chat.tools", "工具")}</span>
              </button>
            ) : (
              <>
                {composerActions}
                <button
                  type="button"
                  onClick={() => {
                    setCapabilityDetailKey(null);
                    setShowCapabilityPicker(true);
                  }}
                  className="inline-flex h-9 items-center gap-1 rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45 sm:h-10 sm:text-[12px]"
                  title={t("chat.capabilityPickerSkills", "技能")}
                  aria-label={t("chat.capabilityPickerSkills", "技能")}
                >
                  <span>{t("chat.capabilityPickerSkills", "技能")}</span>
                </button>
                {teamId && (
                  <>
                <button
                  type="button"
                  onClick={() => setShowDocPicker(true)}
                  className="inline-flex h-9 items-center gap-1 rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45 sm:h-10 sm:text-[12px]"
                  title={t("chat.attachDocuments", "Attach Documents")}
                  aria-label={t("chat.attachDocuments", "Attach Documents")}
                >
                  <span>{t("chat.attachDocumentsShort", "Attach")}</span>
                </button>
                <button
                  type="button"
                  onClick={() => fileInputRef.current?.click()}
                  disabled={uploading}
                  className="inline-flex h-9 items-center gap-1 rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45 disabled:opacity-50 sm:h-10 sm:text-[12px]"
                  title={t("chat.upload", "Upload")}
                  aria-label={t("chat.upload", "Upload")}
                >
                  <span>{t("chat.uploadShort", "Upload")}</span>
                </button>
                  </>
                )}
              </>
            )}
            <input
              ref={fileInputRef}
              type="file"
              accept={FILE_ACCEPT}
              multiple
              className="hidden"
              onChange={handleFileUpload}
            />
          </div>
        )}
        <div className="min-w-0 flex-1">
          <ChatInput
            onSend={handleSend}
            onStop={handleStop}
            isProcessing={isProcessing}
            canSendEmpty={selectedCapabilityRefs.length > 0}
            composeRequest={visibleComposeRequest}
            quickActionGroups={inputQuickActionGroups}
            onFocusChange={setComposerFocused}
            onContentChange={setDraftContent}
            onComposeApplied={(id) => {
              if (localComposeRequest?.id === id) {
                setLocalComposeRequest(null);
              }
            }}
          />
        </div>
      </div>

      <BottomSheetPanel
        open={composerToolsOpen}
        onOpenChange={setComposerToolsOpen}
        title={t("chat.tools", "工具")}
        description={t(
          "chat.toolsHint",
          "Quickly switch sessions, attach documents, or upload material from here without scrolling back to the top.",
        )}
      >
        <div className="space-y-2">
          {composerCollapsedActions}
          <button
            type="button"
            onClick={() => {
              setComposerToolsOpen(false);
              setCapabilityDetailKey(null);
              setShowCapabilityPicker(true);
            }}
            className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
          >
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-foreground">
                {t("chat.capabilityPickerSkills", "技能")}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                {t(
                  "chat.capabilityPickerComposerHint",
                  "Insert the skills and MCP extensions that this agent can actually call into the composer.",
                )}
              </div>
            </div>
          </button>
          {teamId && (
            <>
          <button
            type="button"
            onClick={() => {
              setComposerToolsOpen(false);
              setShowDocPicker(true);
            }}
            className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
          >
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-foreground">
                {t("documents.attachDocuments", "附加文档")}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                {t("chat.attachDocumentsHint", "把团队文档加到当前对话上下文中。")}
              </div>
            </div>
          </button>
          <button
            type="button"
            onClick={() => {
              setComposerToolsOpen(false);
              fileInputRef.current?.click();
            }}
            disabled={uploading}
            className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30 disabled:opacity-50"
          >
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-foreground">
                {t("documents.upload", "上传文件")}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                {t("chat.uploadHint", "上传本地资料，让当前对话直接使用。")}
              </div>
            </div>
          </button>
            </>
          )}
        </div>
      </BottomSheetPanel>

      <ChatCapabilityPicker
        open={showCapabilityPicker}
        onOpenChange={(open) => {
          setShowCapabilityPicker(open);
          if (!open) {
            setCapabilityDetailKey(null);
          }
        }}
        catalog={capabilityCatalog}
        loading={capabilityLoading}
        error={capabilityError}
        initialSelectedKeys={selectedCapabilityKeys}
        initialDetailKey={capabilityDetailKey}
        onInsert={applyCapabilitySelection}
      />

      {/* Document Picker Dialog */}
      {teamId && (
        <DocumentPicker
          teamId={teamId}
          open={showDocPicker}
          onClose={() => setShowDocPicker(false)}
          onSelect={(docs) => {
            setAttachedDocs((prev) => {
              const existingIds = new Set(prev.map((d) => d.id));
              return [...prev, ...docs.filter((d) => !existingIds.has(d.id))];
            });
            setPendingDocIds((prev) => {
              const existingIds = new Set(prev);
              return [
                ...prev,
                ...docs.map((d) => d.id).filter((id) => !existingIds.has(id)),
              ];
            });
          }}
          selectedIds={pendingDocIds}
        />
      )}
    </div>
  );
}
