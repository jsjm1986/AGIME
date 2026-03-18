import { useEffect, useRef, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { MissionStep } from '../../api/mission';
import MarkdownContent from '../MarkdownContent';

interface StreamMessage {
  type: string;
  content: string;
  timestamp: number;
}

interface MissionStepDetailProps {
  step: MissionStep;
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

function normalizeChunk(content: string): string {
  return content.trim().replace(/\s+/g, ' ');
}

/** Detect pure JSON content and wrap it in a markdown code block for rendering. */
function prepareMarkdown(text: string): string {
  const trimmed = text.trim();
  if ((trimmed.startsWith('{') && trimmed.endsWith('}')) ||
      (trimmed.startsWith('[') && trimmed.endsWith(']'))) {
    try {
      const parsed = JSON.parse(trimmed);
      return '```json\n' + JSON.stringify(parsed, null, 2) + '\n```';
    } catch { /* not valid JSON, render as-is */ }
  }
  return text;
}

function humanizeToken(value?: string | null): string {
  if (!value) return '';
  return value.replace(/_/g, ' ').replace(/\b\w/g, ch => ch.toUpperCase());
}

function buildDisplayItems(messages: StreamMessage[]): DisplayItem[] {
  const items: DisplayItem[] = [];
  let i = 0;
  while (i < messages.length) {
    const msg = messages[i];
    if (msg.type === 'text') {
      // Merge consecutive text and drop replayed duplicate chunks.
      const chunks: string[] = [msg.content];
      while (i + 1 < messages.length && messages[i + 1].type === 'text') {
        i++;
        const next = messages[i].content;
        const prev = chunks[chunks.length - 1];
        if (normalizeChunk(next) === normalizeChunk(prev)) {
          continue;
        }
        chunks.push(next);
      }
      const text = chunks.join('');
      if (text.trim()) items.push({ kind: 'text', content: text });
    } else if (msg.type === 'thinking') {
      // Merge consecutive thinking and drop replayed duplicate chunks.
      const chunks: string[] = [msg.content];
      while (i + 1 < messages.length && messages[i + 1].type === 'thinking') {
        i++;
        const next = messages[i].content;
        const prev = chunks[chunks.length - 1];
        if (normalizeChunk(next) === normalizeChunk(prev)) {
          continue;
        }
        chunks.push(next);
      }
      const text = chunks.join('');
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
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <div
      className="cursor-pointer select-none text-xs text-muted-foreground/75"
      onClick={() => setOpen(!open)}
    >
      {open ? (
        <div className="italic whitespace-pre-wrap">{content}</div>
      ) : (
        <span className="hover:text-muted-foreground transition-colors">··· {t('mission.thinking')}</span>
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
          <span className="rounded-full border border-border/60 bg-muted/20 px-2 py-0.5 text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
            Step {step.index + 1}
          </span>
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
          <p className="mt-2 text-sm leading-6 text-muted-foreground/80">{step.description}</p>
        )}
        {(step.required_artifacts?.length || step.completion_checks?.length || step.use_subagent || step.supervisor_state || step.current_blocker || step.last_supervisor_hint || step.evidence_bundle) ? (
          <div className="mt-3 grid gap-2 md:grid-cols-2 xl:grid-cols-3">
            {(step.required_artifacts?.length || step.completion_checks?.length || step.use_subagent) ? (
              <div className="rounded-xl border border-border/55 bg-muted/18 px-3 py-2.5">
                <p className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground/58">{t('mission.contract')}</p>
                <div className="mt-2 space-y-1 text-xs text-muted-foreground/78">
                  {step.required_artifacts && step.required_artifacts.length > 0 && (
                    <p>{t('mission.requiredOutputs')}: {step.required_artifacts.length}</p>
                  )}
                  {step.completion_checks && step.completion_checks.length > 0 && (
                    <p>{t('mission.checks')}: {step.completion_checks.length}</p>
                  )}
                  {step.use_subagent && (
                    <p>{t('mission.delegationEnabled')}</p>
                  )}
                </div>
              </div>
            ) : null}
            {(step.supervisor_state || step.current_blocker || step.last_supervisor_hint) ? (
              <div className="rounded-xl border border-border/55 bg-muted/18 px-3 py-2.5">
                <p className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground/58">{t('mission.supervision')}</p>
                <div className="mt-2 space-y-1 text-xs leading-5 text-muted-foreground/78">
                  {step.supervisor_state && <p>{t('mission.stateLabel')}: {humanizeToken(step.supervisor_state)}</p>}
                  {step.current_blocker && <p>{t('mission.blockerLabel')}: {step.current_blocker}</p>}
                  {step.last_supervisor_hint && <p>{t('mission.hintLabel')}: {step.last_supervisor_hint}</p>}
                </div>
              </div>
            ) : null}
            {step.evidence_bundle ? (
              <div className="rounded-xl border border-border/55 bg-muted/18 px-3 py-2.5">
                <p className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground/58">{t('mission.evidence')}</p>
                <div className="mt-2 grid grid-cols-2 gap-2 text-xs text-muted-foreground/78">
                  <span>{t('mission.artifacts')} {step.evidence_bundle.artifact_paths?.length ?? 0}</span>
                  <span>{t('mission.qualityEvidence')} {step.evidence_bundle.quality_evidence_paths?.length ?? 0}</span>
                  <span>{t('mission.runtimeEvidence')} {step.evidence_bundle.runtime_evidence_paths?.length ?? 0}</span>
                  <span>{t('mission.deploymentEvidence')} {step.evidence_bundle.deployment_evidence_paths?.length ?? 0}</span>
                </div>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>

      {/* Stream output */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto px-4 py-4 space-y-3">
        {messages.length === 0 && !isActive && (
          step.output_summary ? (
            <>
              <MarkdownContent content={prepareMarkdown(step.output_summary)} className="text-sm" />
              {/* Persisted tool calls for completed steps */}
              {step.tool_calls && step.tool_calls.length > 0 && (
                <ToolCallSummary calls={step.tool_calls} />
              )}
            </>
          ) : (
            <div className="flex h-full flex-col items-center justify-center text-muted-foreground/65">
              <span className="text-lg">◇</span>
              <span className="text-xs mt-1">
                {step.status === 'awaiting_approval'
                  ? t('mission.awaitingApproval')
                  : step.status === 'skipped'
                    ? t('mission.skipped')
                    : t(`mission.${step.status}`, humanizeToken(step.status))}
              </span>
            </div>
          )
        )}

        {items.map((item, i) => {
          if (item.kind === 'text') {
            return (
              <MarkdownContent key={i} content={prepareMarkdown(item.content)} className="text-sm" />
            );
          }
          if (item.kind === 'thinking') {
            return <ThinkingBlock key={i} content={item.content} />;
          }
          if (item.kind === 'tool') {
            return (
              <div key={i} className="flex items-center gap-2 font-mono text-xs text-muted-foreground/75">
                <span className="text-muted-foreground/55">↗</span>
                <span>{item.name}</span>
                {item.hasResult ? (
                  <span className="text-muted-foreground/55">✓</span>
                ) : (
                  <span className="text-status-error-text/75">✗</span>
                )}
              </div>
            );
          }
          if (item.kind === 'toolgroup') {
            return (
              <div key={i} className="flex items-center gap-2 font-mono text-xs text-muted-foreground/75">
                <span className="text-muted-foreground/55">↗</span>
                <span>{item.name}</span>
                <span className="text-muted-foreground/55">×{item.count}</span>
                {item.failed > 0 && <span className="text-status-error-text/75">{item.failed} failed</span>}
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

/** Compact summary of persisted tool calls for completed steps. */
function ToolCallSummary({ calls }: { calls: { name: string; success: boolean }[] }) {
  const grouped = useMemo(() => {
    const map = new Map<string, { count: number; failed: number }>();
    for (const c of calls) {
      const entry = map.get(c.name) || { count: 0, failed: 0 };
      entry.count++;
      if (!c.success) entry.failed++;
      map.set(c.name, entry);
    }
    return Array.from(map.entries());
  }, [calls]);

  return (
    <div className="mt-3 pt-3 border-t border-border/30 space-y-1">
      <span className="text-xs text-muted-foreground/70">{calls.length} tool calls</span>
      {grouped.map(([name, { count, failed }]) => (
        <div key={name} className="flex items-center gap-2 font-mono text-xs text-muted-foreground/75">
          <span className="text-muted-foreground/55">↗</span>
          <span>{name}</span>
          {count > 1 && <span className="text-muted-foreground/55">×{count}</span>}
          {failed > 0 && <span className="text-status-error-text/75">{failed} failed</span>}
        </div>
      ))}
    </div>
  );
}
