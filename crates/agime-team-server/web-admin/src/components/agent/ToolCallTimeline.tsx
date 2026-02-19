import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { ToolCall } from './TaskResultParser';

interface Props {
  toolCalls: ToolCall[];
  filter: string | null;
  onFilterChange: (filter: string | null) => void;
}

export function ToolCallTimeline({ toolCalls, filter, onFilterChange }: Props) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  // Get unique tools for filter
  const uniqueTools = Array.from(
    new Set(toolCalls.map(c => `${c.mcpServer}__${c.toolName}`))
  );

  // Filter tool calls
  const filteredCalls = filter
    ? toolCalls.filter(c => `${c.mcpServer}__${c.toolName}` === filter)
    : toolCalls;

  const displayCalls = expanded ? filteredCalls : filteredCalls.slice(0, 5);
  const hasMore = filteredCalls.length > 5;

  if (toolCalls.length === 0) return null;

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span>🔧</span>
          <h4 className="font-medium">
            {t('agent.timeline.title', 'Tool Calls')} ({filteredCalls.length})
          </h4>
        </div>
      </div>

      {/* Filter buttons */}
      {uniqueTools.length > 1 && (
        <div className="flex flex-wrap gap-1">
          <Button
            size="sm"
            variant={filter === null ? 'default' : 'outline'}
            onClick={() => onFilterChange(null)}
            className="h-6 text-xs"
          >
            {t('agent.timeline.all', 'All')}
          </Button>
          {uniqueTools.map(tool => (
            <Button
              key={tool}
              size="sm"
              variant={filter === tool ? 'default' : 'outline'}
              onClick={() => onFilterChange(tool)}
              className="h-6 text-xs"
            >
              {tool.split('__').pop()}
            </Button>
          ))}
        </div>
      )}

      {/* Timeline */}
      <div className="space-y-1 pl-2 border-l-2 border-blue-200">
        {displayCalls.map((call) => (
          <TimelineItem key={call.id} call={call} />
        ))}
      </div>

      {/* Show more button */}
      {hasMore && (
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setExpanded(!expanded)}
          className="w-full text-xs"
        >
          {expanded
            ? t('agent.timeline.showLess', 'Show Less')
            : t('agent.timeline.showMore', `Show ${filteredCalls.length - 5} More`)}
        </Button>
      )}
    </div>
  );
}

function TimelineItem({ call }: { call: ToolCall }) {
  return (
    <div className="flex items-center gap-2 py-1 pl-3 relative">
      <div className="absolute -left-[9px] w-4 h-4 rounded-full bg-blue-100 border-2 border-blue-400 flex items-center justify-center">
        <span className="text-[8px]">✓</span>
      </div>
      <Badge variant="outline" className="text-xs font-mono">
        {call.mcpServer}__{call.toolName}
      </Badge>
      <span className="text-xs text-muted-foreground font-mono">
        {call.callId.slice(0, 12)}...
      </span>
    </div>
  );
}
