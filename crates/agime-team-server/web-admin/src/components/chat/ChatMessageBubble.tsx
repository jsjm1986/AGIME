import { useState, useRef, useEffect } from 'react';
import { ChevronDown, ChevronRight, Wrench, Brain, Bot, Copy, Check } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import MarkdownContent from '../MarkdownContent';
import { formatRelativeTime } from '../../utils/format';

export interface ToolCallInfo {
  name: string;
  id: string;
  result?: string;
  success?: boolean;
}

export interface Message {
  id: string;
  role: 'user' | 'assistant';
  content: string;
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  isStreaming?: boolean;
  timestamp: Date;
}

interface ChatMessageProps {
  role: 'user' | 'assistant';
  content: string;
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  isStreaming?: boolean;
  timestamp?: Date;
  agentName?: string;
  userName?: string;
  autoExpandTools?: boolean;
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
  autoExpandTools = false,
}: ChatMessageProps) {
  const { t } = useTranslation();
  const [showThinking, setShowThinking] = useState(false);
  const [showTools, setShowTools] = useState(false);
  const [copied, setCopied] = useState(false);
  const copyTimeoutRef = useRef<number | null>(null);
  const isUser = role === 'user';

  // Auto-expand live reasoning/tool panels so users can see progress immediately.
  useEffect(() => {
    if (isStreaming && thinking && !showThinking) {
      setShowThinking(true);
    }
  }, [isStreaming, thinking, showThinking]);

  useEffect(() => {
    if (autoExpandTools && isStreaming && toolCalls && toolCalls.length > 0 && !showTools) {
      setShowTools(true);
    }
  }, [autoExpandTools, isStreaming, showTools, toolCalls]);

  useEffect(() => {
    return () => {
      if (copyTimeoutRef.current) window.clearTimeout(copyTimeoutRef.current);
    };
  }, []);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(content);
      setCopied(true);
      if (copyTimeoutRef.current) window.clearTimeout(copyTimeoutRef.current);
      copyTimeoutRef.current = window.setTimeout(() => setCopied(false), 2000);
    } catch { /* ignore */ }
  };

  const avatarLetter = isUser
    ? (userName?.charAt(0) || 'U').toUpperCase()
    : null;
  const toolCallTotal = toolCalls?.length || 0;
  const toolCallSuccess = toolCalls?.filter(tc => tc.success === true).length || 0;
  const toolCallFailed = toolCalls?.filter(tc => tc.success === false).length || 0;
  const toolCallRunning = isStreaming
    ? Math.max(0, toolCallTotal - toolCallSuccess - toolCallFailed)
    : 0;
  const hasRecordedToolOutcome = toolCallSuccess > 0 || toolCallFailed > 0;

  return (
    <div className={`flex gap-3 mb-5 min-w-0 ${isUser ? 'flex-row-reverse' : 'flex-row'}`}>
      {/* Avatar */}
      <div className="shrink-0 mt-0.5">
        {isUser ? (
          <div className="w-8 h-8 rounded-full bg-primary flex items-center justify-center">
            <span className="text-xs font-semibold text-primary-foreground">{avatarLetter}</span>
          </div>
        ) : (
          <div className="w-8 h-8 rounded-full bg-muted-foreground/15 flex items-center justify-center">
            <Bot className="w-4 h-4 text-muted-foreground" />
          </div>
        )}
      </div>

      {/* Message body */}
      <div
        className={`group flex flex-col ${isUser ? 'items-end' : 'items-start'} min-w-0 max-w-[92%] md:max-w-[80%] lg:max-w-[760px]`}
      >
        {/* Sender name */}
        <span className="text-xs text-muted-foreground mb-1 px-1">
          {isUser ? (userName || t('chat.you', 'You')) : (agentName || 'Agent')}
        </span>

        <div
          className={`relative rounded-lg px-4 py-3 ${
            isUser
              ? 'bg-primary text-primary-foreground'
              : 'bg-muted text-foreground'
          } max-w-full min-w-0 overflow-hidden`}
        >
          {/* Thinking section */}
          {thinking && (
            <div className="mb-2 border-l-2 border-purple-400 pl-2">
              <button
                onClick={() => setShowThinking(!showThinking)}
                className="flex items-center gap-1 text-xs opacity-70 hover:opacity-100"
              >
                <Brain className="h-3 w-3" />
                {showThinking ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
                {t('chat.thinking', 'Thinking')}
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
              <div className="whitespace-pre-wrap break-words [overflow-wrap:anywhere] [word-break:break-word]">{content}</div>
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
            <div className="mt-2 border-l-2 border-blue-400 pl-2">
              <button
                onClick={() => setShowTools(!showTools)}
                className="flex items-center gap-1 text-xs opacity-70 hover:opacity-100"
              >
                <Wrench className="h-3 w-3" />
                {showTools ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
                {isStreaming || hasRecordedToolOutcome
                  ? t('chat.toolCallsSummary', '{{count}} tool calls · {{ok}} success · {{failed}} failed · {{running}} running', {
                      count: toolCallTotal,
                      ok: toolCallSuccess,
                      failed: toolCallFailed,
                      running: toolCallRunning,
                    })
                  : t('chat.toolCallsNoStatus', '{{count}} tool calls', {
                      count: toolCallTotal,
                    })}
              </button>
              {showTools && (
                <div className="mt-1 space-y-1">
                  {toolCalls.map((tc) => (
                    <div key={tc.id} className="text-xs">
                      <span className="font-mono font-medium">{tc.name}</span>
                      {tc.result && (
                        <div className={`mt-0.5 opacity-70 truncate max-w-[300px] ${
                          tc.success === false ? 'text-red-400' : ''
                        }`}>
                          {tc.result}
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
            <div className="mt-1 text-xs opacity-50">
              {t('chat.turnProgress', 'Turn {{current}}/{{max}}', { current: turn.current, max: turn.max })}
            </div>
          )}

          {/* Compaction notice */}
          {compaction && (
            <div className="mt-1 text-xs opacity-50 italic">
              {t('chat.contextCompacted', 'Context compacted: {{before}} → {{after}} tokens', { before: compaction.before, after: compaction.after })}
            </div>
          )}

          {/* Copy button (assistant only, on hover) */}
          {!isUser && content && !isStreaming && (
            <button
              onClick={handleCopy}
              className="absolute -bottom-3 right-2 opacity-0 group-hover:opacity-100 transition-opacity
                bg-background border rounded-md p-1 shadow-sm hover:bg-accent"
              title={t('common.copy', 'Copy')}
            >
              {copied
                ? <Check className="h-3.5 w-3.5 text-emerald-500" />
                : <Copy className="h-3.5 w-3.5 text-muted-foreground" />
              }
            </button>
          )}
        </div>

        {/* Timestamp */}
        {timestamp && (
          <span className="text-caption text-muted-foreground/60 mt-1 px-1">
            {formatRelativeTime(timestamp, t)}
          </span>
        )}
      </div>
    </div>
  );
}
