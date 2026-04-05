import { useEffect, useId, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Wrench,
  Brain,
  Bot,
  Copy,
  Check,
  Sparkles,
  Zap,
  Puzzle,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import MarkdownContent from "../MarkdownContent";
import { formatRelativeTime } from "../../utils/format";
import { copyText } from "../../utils/clipboard";

export interface ToolCallInfo {
  name: string;
  id: string;
  result?: string;
  success?: boolean;
  durationMs?: number;
  status?: "running" | "completed" | "failed" | "missing";
}

export interface Message {
  id: string;
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  rawContent?: string;
  rawThinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  isStreaming?: boolean;
  timestamp: Date;
}

interface ChatMessageProps {
  role: "user" | "assistant";
  content: string;
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  isStreaming?: boolean;
  timestamp?: Date;
  agentName?: string;
  userName?: string;
  layoutVariant?: "default" | "workspace";
}

const TOOL_LABELS: Record<string, { zh: string; en: string }> = {
  document_inventory: { zh: "查看文档目录", en: "View document inventory" },
  search_documents: { zh: "搜索文档", en: "Search documents" },
  search_document: { zh: "搜索文档", en: "Search documents" },
  list_documents: { zh: "列出文档", en: "List documents" },
  list_document_versions: { zh: "查看文档版本", en: "List document versions" },
  read_document: { zh: "读取文档", en: "Read document" },
  get_document: { zh: "读取文档", en: "Read document" },
  create_document: { zh: "创建文档", en: "Create document" },
  update_document: { zh: "更新文档", en: "Update document" },
  export_document_to_workspace: {
    zh: "导出文档到工作区",
    en: "Export document to workspace",
  },
  configure_portal_service_agent: {
    zh: "配置分身服务能力",
    en: "Configure avatar service agent",
  },
  get_portal_service_capability_profile: {
    zh: "读取分身服务配置",
    en: "Read avatar capability profile",
  },
  publish_portal: { zh: "发布分身页面", en: "Publish portal page" },
  shell_command: { zh: "执行命令", en: "Run command" },
  search_query: { zh: "搜索网页", en: "Search the web" },
  open: { zh: "打开页面", en: "Open page" },
  click: { zh: "点击页面", en: "Click page" },
  find: { zh: "查找内容", en: "Find content" },
  screenshot: { zh: "截图", en: "Take screenshot" },
  weather: { zh: "查询天气", en: "Get weather" },
  finance: { zh: "查询价格", en: "Get price quote" },
};

const RESULT_VALUE_LABELS: Record<string, { zh: string; en: string }> = {
  document_inventory: { zh: "文档目录", en: "document inventory" },
  controlled_write: { zh: "受控写入", en: "controlled write" },
  co_edit_draft: { zh: "协作草稿", en: "co-edit draft" },
  read_only: { zh: "只读", en: "read-only" },
  public_page: { zh: "正式访客页", en: "public page" },
  preview_only: { zh: "仅预览", en: "preview only" },
  website: { zh: "完整页面", en: "website" },
  widget: { zh: "嵌入挂件", en: "widget" },
  agent_only: { zh: "仅 Agent", en: "agent only" },
};

function clipText(value: string, max = 140) {
  if (value.length <= max) return value;
  return `${value.slice(0, max - 1)}…`;
}

function titleCase(value: string) {
  return value
    .split(" ")
    .map((part) =>
      part ? `${part.charAt(0).toUpperCase()}${part.slice(1)}` : part,
    )
    .join(" ");
}

function normalizeToolName(name: string) {
  const normalized = (name || "").trim();
  const parts = normalized.split("__").filter(Boolean);
  if (parts.length <= 1) {
    return {
      raw: normalized,
      server: "",
      tool: normalized,
      normalizedTool: normalized.toLowerCase(),
    };
  }
  const [server, ...toolParts] = parts;
  const tool = toolParts.join("__");
  return {
    raw: normalized,
    server,
    tool,
    normalizedTool: tool.toLowerCase(),
  };
}

function formatToolLabel(name: string, isZh: boolean) {
  const { tool, normalizedTool } = normalizeToolName(name);
  const preset = TOOL_LABELS[normalizedTool];
  if (preset) {
    return isZh ? preset.zh : preset.en;
  }

  const cleaned = (tool || name || "")
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) {
    return isZh ? "工具" : "Tool";
  }
  return isZh ? cleaned : titleCase(cleaned);
}

function formatValueLabel(value: string, isZh: boolean) {
  const normalized = value.trim().toLowerCase();
  const preset = RESULT_VALUE_LABELS[normalized];
  if (preset) {
    return isZh ? preset.zh : preset.en;
  }
  const cleaned = value.replace(/[_-]+/g, " ").replace(/\s+/g, " ").trim();
  if (!cleaned) return value;
  return isZh ? cleaned : cleaned.toLowerCase();
}

function tryParseStructuredResult(raw: string): unknown | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    try {
      return JSON.parse(trimmed);
    } catch {
      return null;
    }
  }
  return null;
}

function readFirstString(
  record: Record<string, unknown>,
  keys: string[],
): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

function readFirstArray(
  record: Record<string, unknown>,
  keys: string[],
): unknown[] | null {
  for (const key of keys) {
    const value = record[key];
    if (Array.isArray(value)) {
      return value;
    }
  }
  return null;
}

function readFirstCount(
  record: Record<string, unknown>,
  keys: string[],
): number | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
    if (Array.isArray(value)) {
      return value.length;
    }
  }
  return null;
}

function formatToolDuration(
  durationMs: number | undefined,
  t: ReturnType<typeof useTranslation>["t"],
) {
  if (!durationMs || !Number.isFinite(durationMs) || durationMs <= 0) {
    return null;
  }
  if (durationMs < 1000) {
    return t("chat.toolDurationMs", "{{n}}ms", { n: durationMs });
  }
  const seconds = durationMs / 1000;
  return t("chat.toolDurationSeconds", "{{n}}s", {
    n: seconds >= 10 ? seconds.toFixed(0) : seconds.toFixed(1),
  });
}

function formatRawToolResult(raw: string) {
  const parsed = tryParseStructuredResult(raw);
  if (parsed !== null) {
    try {
      return JSON.stringify(parsed, null, 2);
    } catch {
      return raw;
    }
  }
  return raw;
}

function summarizeToolResult(
  rawResult: string | undefined,
  success: boolean | undefined,
  status: ToolCallInfo["status"],
  toolName: string,
  isZh: boolean,
  t: ReturnType<typeof useTranslation>["t"],
) {
  const result = (rawResult || "").trim();
  if (!result) {
    if (status === "missing") {
      return t(
        "chat.toolMissingResult",
        "Completed, but no persisted tool result was recorded",
      );
    }
    if (success === false) {
      return t(
        "chat.toolFailedNoDetails",
        "Execution failed with no additional details",
      );
    }
    if (success === true) {
      return t(
        "chat.toolCompletedNoResult",
        "Completed without a readable summary",
      );
    }
    return t("chat.toolWaitingResult", "Waiting for the tool result");
  }

  const parsed = tryParseStructuredResult(result);
  if (Array.isArray(parsed)) {
    return t("chat.toolReturnedItems", "Returned {{count}} item(s)", {
      count: parsed.length,
    });
  }

  if (parsed && typeof parsed === "object") {
    const record = parsed as Record<string, unknown>;
    const note = readFirstString(record, [
      "summary",
      "message",
      "note",
      "detail",
      "description",
    ]);
    if (note) {
      return clipText(note, 160);
    }

    const errorText = readFirstString(record, ["error", "reason"]);
    if (errorText) {
      return clipText(errorText, 160);
    }

    const view = readFirstString(record, ["view", "mode", "status", "action"]);
    if (view) {
      return t("chat.toolReturnedView", "Returned {{name}}", {
        name: formatValueLabel(view, isZh),
      });
    }

    const items = readFirstArray(record, [
      "items",
      "results",
      "documents",
      "files",
    ]);
    if (items) {
      return t("chat.toolReturnedItems", "Returned {{count}} item(s)", {
        count: items.length,
      });
    }

    const count = readFirstCount(record, [
      "count",
      "total",
      "matched",
      "created",
      "updated",
      "deleted",
    ]);
    if (count !== null) {
      return t("chat.toolReturnedItems", "Returned {{count}} item(s)", {
        count,
      });
    }

    const valueText = readFirstString(record, [
      "value",
      "path",
      "url",
      "file",
      "filename",
    ]);
    if (valueText) {
      return clipText(valueText, 160);
    }

    return t("chat.toolReturnedStructuredResult", "Returned structured result");
  }

  const oneLine = result.replace(/\s+/g, " ").trim();
  if (!oneLine) {
    return t("chat.toolReturnedStructuredResult", "Returned structured result");
  }

  const displayName = formatToolLabel(toolName, isZh);
  if (oneLine.toLowerCase() === "ok" || oneLine.toLowerCase() === "done") {
    return t("chat.toolDoneBy", "{{name}} completed", { name: displayName });
  }
  return clipText(oneLine, 160);
}

function buildToolCallsSummary(
  count: number,
  failed: number,
  running: number,
  missing: number,
  t: ReturnType<typeof useTranslation>["t"],
) {
  if (failed > 0 && running > 0) {
    return t(
      "chat.toolCallsSummaryFailedAndRunning",
      "{{count}} tool calls, {{failed}} failed, {{running}} running",
      { count, failed, running },
    );
  }
  if (missing > 0 && running > 0) {
    return t(
      "chat.toolCallsSummaryMissingAndRunning",
      "{{count}} tool calls, {{missing}} missing result(s), {{running}} running",
      { count, missing, running },
    );
  }
  if (failed > 0) {
    return t(
      "chat.toolCallsSummaryFailed",
      "{{count}} tool calls, {{failed}} failed",
      {
        count,
        failed,
      },
    );
  }
  if (missing > 0) {
    return t(
      "chat.toolCallsSummaryMissing",
      "{{count}} tool calls, {{missing}} missing result(s)",
      {
        count,
        missing,
      },
    );
  }
  if (running > 0) {
    return t(
      "chat.toolCallsSummaryRunning",
      "{{count}} tool calls, {{running}} running",
      {
        count,
        running,
      },
    );
  }
  return t("chat.toolCallsNoStatus", "{{count}} tool call(s)", { count });
}

const CAPABILITY_BLOCK_HEADER = "请优先使用以下能力完成本轮任务：";

interface ParsedCapabilityRef {
  ref: string;
  kind: "skill" | "extension";
  name: string;
}

function parseCapabilityNameFromRef(ref: string): string {
  const parts = ref
    .replace(/^\[\[/, "")
    .replace(/\]\]$/, "")
    .split("|");
  return parts[1] || ref;
}

function parseCapabilityBlock(text: string): {
  refs: ParsedCapabilityRef[];
  remainder: string;
  hasBlock: boolean;
} {
  const normalized = text.replace(/\r\n/g, "\n");
  if (!normalized.startsWith(CAPABILITY_BLOCK_HEADER)) {
    return { refs: [], remainder: text, hasBlock: false };
  }

  const lines = normalized.split("\n");
  let index = 1;
  const refs: ParsedCapabilityRef[] = [];

  while (index < lines.length) {
    const match = lines[index].match(
      /^\s*-\s*(\[\[(skill|ext):.+?\]\])\s*$/i,
    );
    if (!match) {
      break;
    }
    const ref = match[1];
    refs.push({
      ref,
      kind: match[2].toLowerCase() === "skill" ? "skill" : "extension",
      name: parseCapabilityNameFromRef(ref),
    });
    index += 1;
  }

  while (index < lines.length && lines[index].trim() === "") {
    index += 1;
  }

  return {
    refs,
    remainder: lines.slice(index).join("\n"),
    hasBlock: refs.length > 0,
  };
}

function InfinityLoopStatus({
  current,
  max,
  label,
  animated,
}: {
  current: number;
  max: number;
  label: string;
  animated: boolean;
}) {
  const gradientId = useId().replace(/:/g, "");
  const motionPath =
    "M4,12 C4,7 8.5,5 12,5 C16.5,5 19.5,9 24,12 C28.5,15 31.5,19 36,19 C39.5,19 44,17 44,12 C44,7 39.5,5 36,5 C31.5,5 28.5,9 24,12 C19.5,15 16.5,19 12,19 C8.5,19 4,17 4,12";
  const dashPath =
    "M4,12 C4,7 8.5,5 12,5 C16.5,5 19.5,9 24,12 C28.5,15 31.5,19 36,19 C39.5,19 44,17 44,12 C44,7 39.5,5 36,5 C31.5,5 28.5,9 24,12 C19.5,15 16.5,19 12,19 C8.5,19 4,17 4,12";
  const loopScale = max > 0 ? Math.min(1.1, Math.max(0.92, current / max)) : 1;

  return (
    <div
      className="mt-2 inline-flex items-center justify-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.75] px-2 py-1 text-[hsl(var(--primary))]"
      title={label}
      aria-label={label}
    >
      <span className="sr-only">{label}</span>
      <svg
        viewBox="0 0 48 24"
        className="h-4 w-10 overflow-visible"
        aria-hidden="true"
      >
        <defs>
          <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="currentColor" stopOpacity="0.22" />
            <stop offset="50%" stopColor="currentColor" stopOpacity="0.92" />
            <stop offset="100%" stopColor="currentColor" stopOpacity="0.22" />
          </linearGradient>
        </defs>
        <path
          d={dashPath}
          fill="none"
          stroke="currentColor"
          strokeWidth="2.25"
          strokeLinecap="round"
          opacity="0.14"
        />
        <g transform={`translate(24 12) scale(${loopScale}) translate(-24 -12)`}>
          <path
            d={dashPath}
            fill="none"
            stroke={`url(#${gradientId})`}
            strokeWidth="2.35"
            strokeLinecap="round"
            strokeDasharray={animated ? "18 14" : "44 0"}
          >
            {animated ? (
              <>
                <animate
                  attributeName="stroke-dashoffset"
                  values="0;-64"
                  dur="1.65s"
                  repeatCount="indefinite"
                />
                <animate
                  attributeName="opacity"
                  values="0.55;1;0.55"
                  dur="1.65s"
                  repeatCount="indefinite"
                />
              </>
            ) : (
              <animate
                attributeName="opacity"
                values="0.82"
                dur="0.01s"
                fill="freeze"
              />
            )}
          </path>
          {animated ? (
            <circle r="1.8" fill="currentColor" opacity="0.95">
              <animateMotion
                dur="1.65s"
                repeatCount="indefinite"
                rotate="auto"
                path={motionPath}
              />
              <animate
                attributeName="opacity"
                values="0.4;1;0.4"
                dur="1.65s"
                repeatCount="indefinite"
              />
            </circle>
          ) : (
            <>
              <circle cx="12" cy="12" r="1.6" fill="currentColor" opacity="0.72" />
              <circle cx="36" cy="12" r="1.6" fill="currentColor" opacity="0.72" />
            </>
          )}
        </g>
      </svg>
    </div>
  );
}

export function ChatMessageBubble({
  role,
  content,
  thinking,
  toolCalls,
  turn,
  compaction,
  isStreaming,
  timestamp,
  agentName,
  userName,
  layoutVariant = "default",
}: ChatMessageProps) {
  const { t, i18n } = useTranslation();
  const [showThinking, setShowThinking] = useState(false);
  const [showTools, setShowTools] = useState(false);
  const [expandedToolResults, setExpandedToolResults] = useState<
    Record<string, boolean>
  >({});
  const [copied, setCopied] = useState(false);
  const copyTimeoutRef = useRef<number | null>(null);
  const isUser = role === "user";
  const isZh = (i18n.resolvedLanguage || i18n.language || "").startsWith("zh");

  // Auto-expand live reasoning/tool panels so users can see progress immediately.
  useEffect(() => {
    if (isStreaming && thinking && !showThinking) {
      setShowThinking(true);
    }
  }, [isStreaming, thinking, showThinking]);

  useEffect(() => {
    return () => {
      if (copyTimeoutRef.current) window.clearTimeout(copyTimeoutRef.current);
    };
  }, []);

  const handleCopy = async () => {
    if (await copyText(content)) {
      setCopied(true);
      if (copyTimeoutRef.current) window.clearTimeout(copyTimeoutRef.current);
      copyTimeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    }
  };

  const avatarLetter = isUser
    ? (userName?.charAt(0) || "U").toUpperCase()
    : null;
  const toolCallTotal = toolCalls?.length || 0;
  const toolCallSuccess =
    toolCalls?.filter(
      (tc) => tc.success === true || tc.status === "completed",
    ).length || 0;
  const toolCallFailed =
    toolCalls?.filter(
      (tc) => tc.success === false || tc.status === "failed",
    ).length || 0;
  const toolCallMissing =
    toolCalls?.filter((tc) => tc.status === "missing").length || 0;
  const explicitRunning =
    toolCalls?.filter((tc) => tc.status === "running").length || 0;
  const toolCallRunning =
    explicitRunning > 0
      ? explicitRunning
      : isStreaming
        ? Math.max(
            0,
            toolCallTotal - toolCallSuccess - toolCallFailed - toolCallMissing,
          )
        : 0;
  const capabilityBlock = isUser ? parseCapabilityBlock(content) : null;
  const visibleUserContent =
    capabilityBlock?.hasBlock ? capabilityBlock.remainder : content;

  const bubbleWidthClass =
    layoutVariant === "workspace"
      ? "max-w-[95%] md:max-w-[88%] xl:max-w-[76%]"
      : "max-w-[92%] md:max-w-[80%] lg:max-w-[760px]";

  return (
    <div
      className={`flex gap-3 mb-5 min-w-0 w-full ${isUser ? "flex-row-reverse" : "flex-row"}`}
    >
      {/* Avatar */}
      <div className="shrink-0 mt-0.5">
        {isUser ? (
          <div className="w-8 h-8 rounded-full bg-primary flex items-center justify-center">
            <span className="text-xs font-semibold text-primary-foreground">
              {avatarLetter}
            </span>
          </div>
        ) : (
          <div className="w-8 h-8 rounded-full bg-muted-foreground/15 flex items-center justify-center">
            <Bot className="w-4 h-4 text-muted-foreground" />
          </div>
        )}
      </div>

      {/* Message body */}
      <div
        className={`group flex flex-col ${isUser ? "items-end" : "items-start"} min-w-0 ${bubbleWidthClass}`}
      >
        {/* Sender name */}
        <span className="text-xs text-muted-foreground mb-1 px-1">
          {isUser ? userName || t("chat.you", "You") : agentName || "Agent"}
        </span>

        <div
          className={`relative rounded-lg px-4 py-3 ${
            isUser
              ? "bg-primary text-primary-foreground"
              : "bg-[hsl(var(--ui-surface-panel-muted))/0.92] text-foreground"
          } max-w-full min-w-0 ${
            !isUser && content && !isStreaming
              ? "mb-3 overflow-visible"
              : "overflow-hidden"
          }`}
        >
          {/* Copy button (assistant only) */}
          {!isUser && content && !isStreaming && (
            <button
              onClick={handleCopy}
              className="absolute -bottom-3 right-2 z-10 inline-flex h-7 w-7 items-center justify-center rounded-full border border-border/70 bg-background/92 text-muted-foreground opacity-0 shadow-sm transition-opacity hover:bg-accent group-hover:opacity-100 focus-visible:opacity-100"
              title={t("common.copy", "Copy")}
            >
              {copied ? (
                <Check className="h-3.5 w-3.5 text-status-success-text" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
            </button>
          )}

          {/* Thinking section */}
          {thinking && (
            <div className="mb-2 border-l-2 border-[hsl(var(--status-info-text))/0.28] pl-2">
              <button
                onClick={() => setShowThinking(!showThinking)}
                className="flex items-center gap-1 text-xs opacity-70 hover:opacity-100"
              >
                <Brain className="h-3 w-3" />
                {showThinking ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
                {t("chat.thinking", "Thinking")}
              </button>
              {showThinking && (
                <div className="mt-1 text-xs opacity-70 whitespace-pre-wrap break-words [overflow-wrap:anywhere] [word-break:break-word]">
                  {thinking}
                </div>
              )}
            </div>
          )}

          {/* Main content */}
          <div className="min-w-0 max-w-full break-words [overflow-wrap:anywhere] [word-break:break-word] text-[13px] leading-5">
            {isUser ? (
              <>
                {capabilityBlock?.hasBlock && capabilityBlock.refs.length > 0 && (
                  <div className="mb-3 rounded-[18px] border border-white/18 bg-white/10 px-3 py-2.5 backdrop-blur-sm">
                    <div className="mb-2 flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-[0.14em] text-primary-foreground/72">
                      <Sparkles className="h-3.5 w-3.5" />
                      <span>{t("chat.selectedCapabilitiesCard", "已选能力")}</span>
                    </div>
                    <div className="flex flex-wrap gap-1.5">
                      {capabilityBlock.refs.map((item) => (
                        <span
                          key={item.ref}
                          className="inline-flex items-center gap-1.5 rounded-full border border-white/16 bg-white/10 px-2.5 py-1 text-[11px] font-medium text-primary-foreground shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]"
                        >
                          {item.kind === "skill" ? (
                            <Zap className="h-3.5 w-3.5 text-primary-foreground/78" />
                          ) : (
                            <Puzzle className="h-3.5 w-3.5 text-primary-foreground/78" />
                          )}
                          <span className="max-w-[180px] truncate">
                            {item.name}
                          </span>
                        </span>
                      ))}
                    </div>
                  </div>
                )}
                {visibleUserContent.trim().length > 0 && (
                  <div className="whitespace-pre-wrap break-words [overflow-wrap:anywhere] [word-break:break-word]">
                    {visibleUserContent}
                  </div>
                )}
              </>
            ) : (
              <MarkdownContent
                content={content}
                className="text-[13px] leading-5 prose-p:leading-5 prose-table:text-[13px] prose-headings:text-[13px] prose-h1:text-[13px] prose-h2:text-[13px] prose-h3:text-[13px] prose-h1:my-1 prose-h2:my-1 prose-h3:my-1"
              />
            )}
            {isStreaming && <span className="animate-pulse">▊</span>}
          </div>

          {/* Tool calls section */}
          {toolCalls && toolCalls.length > 0 && (
            <div className="mt-2 border-l-2 border-[hsl(var(--status-warning-text))/0.28] pl-2">
              <button
                onClick={() => setShowTools(!showTools)}
                className="flex items-center gap-1 text-xs opacity-70 hover:opacity-100"
              >
                <Wrench className="h-3 w-3" />
                {showTools ? (
                  <ChevronDown className="h-3 w-3" />
                ) : (
                  <ChevronRight className="h-3 w-3" />
                )}
                {buildToolCallsSummary(
                  toolCallTotal,
                  toolCallFailed,
                  toolCallRunning,
                  toolCallMissing,
                  t,
                )}
              </button>
              {showTools && (
                <div className="mt-1.5 space-y-2">
                  {toolCalls.map((tc) => (
                    <div
                      key={tc.id}
                      className="rounded-md border border-border/45 bg-background/35 px-2.5 py-2"
                    >
                      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[12px] leading-5">
                        <span className="font-medium text-foreground">
                          {formatToolLabel(tc.name, isZh)}
                        </span>
                        <span
                          className={`inline-flex rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                            tc.status === "failed" || tc.success === false
                              ? "bg-status-error-bg text-status-error-text"
                              : tc.status === "completed" || tc.success === true
                                ? "bg-status-success-bg text-status-success-text"
                                : tc.status === "missing"
                                  ? "bg-muted text-muted-foreground"
                                : "bg-status-warning-bg text-status-warning-text"
                          }`}
                        >
                          {tc.status === "failed" || tc.success === false
                            ? t("chat.toolStatusFailed", "Failed")
                            : tc.status === "completed" || tc.success === true
                              ? t("chat.toolStatusSuccess", "Completed")
                              : tc.status === "missing"
                                ? t("chat.toolStatusMissing", "No result")
                              : t("chat.toolStatusRunning", "Running")}
                        </span>
                        {formatToolDuration(tc.durationMs, t) && (
                          <span className="text-[11px] text-muted-foreground">
                            {formatToolDuration(tc.durationMs, t)}
                          </span>
                        )}
                      </div>
                      <div
                        className={`mt-1 text-[12px] leading-5 break-words ${
                          tc.success === false
                            ? "text-status-error-text"
                            : "text-muted-foreground"
                        }`}
                      >
                        {summarizeToolResult(
                          tc.result,
                          tc.success,
                          tc.status,
                          tc.name,
                          isZh,
                          t,
                        )}
                      </div>
                      {tc.result && tc.result.trim().length > 0 && (
                        <div className="mt-1.5">
                          <button
                            type="button"
                            className="text-[11px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                            onClick={() =>
                              setExpandedToolResults((prev) => ({
                                ...prev,
                                [tc.id]: !prev[tc.id],
                              }))
                            }
                          >
                            {expandedToolResults[tc.id]
                              ? t("chat.toolHideRawResult", "Hide raw result")
                              : t("chat.toolShowRawResult", "View raw result")}
                          </button>
                          {expandedToolResults[tc.id] && (
                            <div className="mt-2 overflow-hidden rounded-md border border-border/55 bg-background/55">
                              <div className="border-b border-border/55 px-2.5 py-1 text-[10px] font-medium uppercase tracking-[0.08em] text-muted-foreground">
                                {t("chat.toolRawResult", "Raw result")}
                              </div>
                              <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-words px-2.5 py-2 text-[11px] leading-5 text-muted-foreground">
                                {formatRawToolResult(tc.result)}
                              </pre>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}

          {/* Turn progress */}
          {turn && (
            <InfinityLoopStatus
              current={turn.current}
              max={turn.max}
              animated={Boolean(isStreaming)}
              label={t("chat.turnProgress", "Turn {{current}}/{{max}}", {
                current: turn.current,
                max: turn.max > 0 ? turn.max : "∞",
              })}
            />
          )}

          {/* Compaction notice */}
          {compaction && (
            <div className="mt-1 text-xs opacity-50 italic">
              {t(
                "chat.contextCompacted",
                "Context compacted: {{before}} → {{after}} tokens",
                { before: compaction.before, after: compaction.after },
              )}
            </div>
          )}

        </div>

        {/* Timestamp */}
        {timestamp && (
          <span className="mt-1 px-1 text-caption text-muted-foreground/75">
            {formatRelativeTime(timestamp, t)}
          </span>
        )}
      </div>
    </div>
  );
}
