import { useEffect, useRef, useMemo } from 'react';
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

/** Group consecutive text messages into single blocks for readable rendering */
interface GroupedMessage {
  type: string;
  content: string;
}

function groupMessages(messages: StreamMessage[]): GroupedMessage[] {
  const groups: GroupedMessage[] = [];
  for (const msg of messages) {
    const last = groups[groups.length - 1];
    // Accumulate consecutive text messages (not thinking/toolcall/toolresult)
    if (
      msg.type === 'text' &&
      last &&
      last.type === 'text'
    ) {
      last.content += msg.content;
    } else {
      groups.push({ type: msg.type, content: msg.content });
    }
  }
  return groups;
}

export function MissionStepDetail({
  step,
  isActive,
  messages,
}: MissionStepDetailProps) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement>(null);

  const grouped = useMemo(() => groupMessages(messages), [messages]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  return (
    <div className="flex flex-col h-full">
      {/* Step header */}
      <div className="p-3 border-b">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold">
            {t('mission.steps')}: {step.index + 1}
          </span>
          <span className="text-sm font-medium">{step.title}</span>
          {isActive && (
            <span className="ml-auto flex items-center gap-1 text-xs text-blue-600">
              <span className="w-2 h-2 rounded-full bg-blue-600 animate-pulse" />
              {t('mission.running')}
            </span>
          )}
        </div>
        <p className="text-xs text-muted-foreground mt-1">{step.description}</p>
      </div>

      {/* Stream output */}
      <div ref={scrollRef} className="flex-1 overflow-y-auto p-3 space-y-2">
        {messages.length === 0 && !isActive && (
          step.output_summary ? (
            <div className="text-sm whitespace-pre-wrap">{step.output_summary}</div>
          ) : (
            <p className="text-sm text-muted-foreground text-center py-4">
              {step.status === 'pending'
                ? t('mission.pending', 'Pending')
                : t('mission.completed')}
            </p>
          )
        )}
        {grouped.map((msg, i) => (
          <div key={i} className="text-sm">
            {msg.type === 'thinking' ? (
              <div className="text-muted-foreground italic text-xs bg-muted/50 rounded p-2">
                {msg.content}
              </div>
            ) : msg.type === 'toolcall' ? (
              <div className="text-xs font-mono bg-muted rounded p-2 border-l-2 border-blue-500">
                {msg.content}
              </div>
            ) : msg.type === 'toolresult' ? (
              <div className="text-xs font-mono bg-muted rounded p-2 border-l-2 border-green-500 max-h-32 overflow-y-auto">
                {msg.content}
              </div>
            ) : (
              <div className="whitespace-pre-wrap">{msg.content}</div>
            )}
          </div>
        ))}
        {isActive && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <span className="w-1.5 h-1.5 rounded-full bg-blue-500 animate-pulse" />
            {t('mission.running')}...
          </div>
        )}
      </div>
    </div>
  );
}
