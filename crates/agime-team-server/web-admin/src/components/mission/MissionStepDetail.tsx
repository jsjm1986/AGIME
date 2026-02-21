import { useEffect, useRef, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { MissionStep } from '../../api/mission';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

interface MissionStepDetailProps {
  step: MissionStep;
  missionId: string;
  isActive: boolean;
  messages: StreamMessage[];
}

// Grouped display types
type DisplayItem =
  | { kind: 'text'; content: string }
  | { kind: 'thinking'; content: string }
  | { kind: 'tool'; name: string; hasResult: boolean }
  | { kind: 'toolgroup'; name: string; count: number; failed: number };

function parseToolName(raw: string): string {
  try {
    const obj = JSON.parse(raw);
    return obj.name || obj.tool_name || raw;
  } catch {
    return raw;
  }
}

function buildDisplayItems(messages: StreamMessage[]): DisplayItem[] {
  const items: DisplayItem[] = [];
  let i = 0;
  while (i < messages.length) {
    const msg = messages[i];
    if (msg.type === 'text') {
      // Merge consecutive text
      let text = msg.content;
      while (i + 1 < messages.length && messages[i + 1].type === 'text') {
        i++;
        text += messages[i].content;
      }
      if (text.trim()) items.push({ kind: 'text', content: text });
    } else if (msg.type === 'thinking') {
      // Merge consecutive thinking
      let text = msg.content;
      while (i + 1 < messages.length && messages[i + 1].type === 'thinking') {
        i++;
        text += messages[i].content;
      }
      if (text.trim()) items.push({ kind: 'thinking', content: text });
    } else if (msg.type === 'toolcall') {
      const name = parseToolName(msg.content);
      const hasResult = i + 1 < messages.length && messages[i + 1].type === 'toolresult';
      if (hasResult) i++; // skip toolresult
      items.push({ kind: 'tool', name, hasResult });
    } else if (msg.type === 'toolresult') {
      // orphan toolresult, skip
    } else {
      // goal_start, goal_complete, pivot etc - show as text
      if (msg.content.trim()) items.push({ kind: 'text', content: msg.content });
    }
    i++;
  }

  // Merge consecutive same-name tools into toolgroup
  const merged: DisplayItem[] = [];
  for (const item of items) {
    if (item.kind === 'tool') {
      const last = merged[merged.length - 1];
      if (last && last.kind === 'toolgroup' && last.name === item.name) {
        last.count++;
        if (!item.hasResult) last.failed++;
      } else if (last && last.kind === 'tool' && last.name === item.name) {
        merged[merged.length - 1] = {
          kind: 'toolgroup',
          name: item.name,
          count: 2,
          failed: (!last.hasResult ? 1 : 0) + (!item.hasResult ? 1 : 0),
        };
      } else {
        merged.push(item);
      }
    } else {
      merged.push(item);
    }
  }
  return merged;
}

function ThinkingBlock({ content }: { content: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div
      className="text-xs text-muted-foreground/60 cursor-pointer select-none"
      onClick={() => setOpen(!open)}
    >
      {open ? (
        <div className="italic whitespace-pre-wrap">{content}</div>
      ) : (
        <span className="hover:text-muted-foreground transition-colors">··· 思考中</span>
      )}
    </div>
  );
}

export function MissionStepDetail({
  step,
  isActive,
  messages,
}: MissionStepDetailProps) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);
  const items = useMemo(() => buildDisplayItems(messages), [messages]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  return (
    <div className="flex flex-col h-full">
      {/* Step header */}
      <div className="px-4 py-3 border-b border-border/50">
        <div className="flex items-center gap-2">
          <span className="text-xs font-medium text-muted-foreground">Step {step.index + 1}</span>
          <span className="text-xs text-muted-foreground/40">·</span>
          <span className="text-sm font-semibold">{step.title}</span>
          <span className="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
            {step.retry_count > 0 && (
              <span>{t('mission.retryCount', { current: step.retry_count, max: step.max_retries })}</span>
            )}
            {isActive && (
              <span className="flex items-center gap-1.5">
                <span className="w-1.5 h-1.5 rounded-full bg-foreground/60 animate-pulse" />
                {t('mission.running')}
              </span>
            )}
          </span>
        </div>
        {step.description && (
          <p className="text-xs text-muted-foreground/60 mt-1">{step.description}</p>
        )}
      </div>

      {/* Stream output */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-4 space-y-3">
        {messages.length === 0 && !isActive && (
          step.output_summary ? (
            <div className="text-sm whitespace-pre-wrap leading-relaxed">{step.output_summary}</div>
          ) : (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground/40">
              <span className="text-lg">◇</span>
              <span className="text-xs mt-1">
                {step.status === 'pending' ? t('mission.pending', 'Pending') : t('mission.completed')}
              </span>
            </div>
          )
        )}

        {items.map((item, i) => {
          if (item.kind === 'text') {
            return (
              <div key={i} className="text-sm whitespace-pre-wrap leading-relaxed">
                {item.content}
              </div>
            );
          }
          if (item.kind === 'thinking') {
            return <ThinkingBlock key={i} content={item.content} />;
          }
          if (item.kind === 'tool') {
            return (
              <div key={i} className="flex items-center gap-2 text-xs text-muted-foreground/70 font-mono">
                <span className="text-muted-foreground/40">↗</span>
                <span>{item.name}</span>
                {item.hasResult ? (
                  <span className="text-muted-foreground/40">✓</span>
                ) : (
                  <span className="text-red-400/70">✗</span>
                )}
              </div>
            );
          }
          if (item.kind === 'toolgroup') {
            return (
              <div key={i} className="flex items-center gap-2 text-xs text-muted-foreground/70 font-mono">
                <span className="text-muted-foreground/40">↗</span>
                <span>{item.name}</span>
                <span className="text-muted-foreground/40">×{item.count}</span>
                {item.failed > 0 && <span className="text-red-400/70">{item.failed} failed</span>}
              </div>
            );
          }
          return null;
        })}

        {/* Typing cursor */}
        {isActive && (
          <span className="inline-block w-0.5 h-4 bg-foreground/50 animate-pulse" />
        )}
      </div>
    </div>
  );
}
