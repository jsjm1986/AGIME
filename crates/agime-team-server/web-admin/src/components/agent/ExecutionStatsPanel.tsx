import { useTranslation } from 'react-i18next';
import { Card, CardContent } from '../ui/card';
import { Badge } from '../ui/badge';
import { ExecutionStats, formatDuration } from './TaskResultParser';

interface Props {
  stats: ExecutionStats;
}

export function ExecutionStatsPanel({ stats }: Props) {
  const { t } = useTranslation();

  // Sort tools by usage count
  const sortedTools = Object.entries(stats.toolUsage)
    .sort((a, b) => b[1] - a[1]);

  const statTiles = [
    {
      value: stats.totalToolCalls,
      label: t('agent.stats.toolCalls', 'Tool Calls'),
      valueClass: 'text-[hsl(var(--status-info-text))]',
    },
    {
      value: stats.mcpServers.length,
      label: t('agent.stats.mcpServers', 'MCP Servers'),
      valueClass: 'text-[hsl(var(--status-success-text))]',
    },
    {
      value: Object.keys(stats.toolUsage).length,
      label: t('agent.stats.uniqueTools', 'Unique Tools'),
      valueClass: 'text-[hsl(var(--status-warning-text))]',
    },
    {
      value: formatDuration(stats.duration),
      label: t('agent.stats.duration', 'Duration'),
      valueClass: 'text-[hsl(var(--status-error-text))]',
    },
  ];

  return (
    <Card className="border-[hsl(var(--ui-line-soft))/0.82] bg-[linear-gradient(135deg,hsl(var(--ui-surface-panel))/0.98,hsla(var(--ui-surface-selected)/0.82))] shadow-[0_20px_48px_-38px_hsl(var(--foreground)/0.24)]">
      <CardContent className="p-4">
        <div className="flex items-center gap-2 mb-3">
          <span className="text-lg">📊</span>
          <h4 className="font-semibold">{t('agent.stats.title', 'Execution Statistics')}</h4>
        </div>

        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-4">
          {statTiles.map((tile) => (
            <div
              key={tile.label}
              className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel))/0.76] px-3 py-3 text-center shadow-[inset_0_1px_0_hsl(var(--ui-surface-panel-strong)/0.42)]"
            >
              <div className={`text-2xl font-semibold tracking-tight ${tile.valueClass}`}>
                {tile.value}
              </div>
              <div className="mt-1 text-xs text-muted-foreground">
                {tile.label}
              </div>
            </div>
          ))}
        </div>

        {/* Tool Usage Breakdown */}
        {sortedTools.length > 0 && (
          <div className="space-y-2">
            <div className="text-sm font-medium text-muted-foreground">
              {t('agent.stats.toolBreakdown', 'Tool Usage')}
            </div>
            <div className="flex flex-wrap gap-2">
              {sortedTools.map(([tool, count]) => (
                <Badge key={tool} variant="secondary" className="text-xs">
                  🔧 {tool.split('__').pop()}: {count}
                </Badge>
              ))}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}
