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

  return (
    <Card className="bg-gradient-to-r from-blue-50 to-indigo-50 dark:from-blue-950/30 dark:to-indigo-950/30 border-blue-200 dark:border-blue-800">
      <CardContent className="p-4">
        <div className="flex items-center gap-2 mb-3">
          <span className="text-lg">📊</span>
          <h4 className="font-semibold">{t('agent.stats.title', 'Execution Statistics')}</h4>
        </div>

        <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-4">
          {/* Total Tool Calls */}
          <div className="text-center p-2 bg-white/50 dark:bg-black/20 rounded-lg">
            <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">
              {stats.totalToolCalls}
            </div>
            <div className="text-xs text-muted-foreground">
              {t('agent.stats.toolCalls', 'Tool Calls')}
            </div>
          </div>

          {/* MCP Servers */}
          <div className="text-center p-2 bg-white/50 dark:bg-black/20 rounded-lg">
            <div className="text-2xl font-bold text-green-600 dark:text-green-400">
              {stats.mcpServers.length}
            </div>
            <div className="text-xs text-muted-foreground">
              {t('agent.stats.mcpServers', 'MCP Servers')}
            </div>
          </div>

          {/* Unique Tools */}
          <div className="text-center p-2 bg-white/50 dark:bg-black/20 rounded-lg">
            <div className="text-2xl font-bold text-purple-600 dark:text-purple-400">
              {Object.keys(stats.toolUsage).length}
            </div>
            <div className="text-xs text-muted-foreground">
              {t('agent.stats.uniqueTools', 'Unique Tools')}
            </div>
          </div>

          {/* Duration */}
          <div className="text-center p-2 bg-white/50 dark:bg-black/20 rounded-lg">
            <div className="text-2xl font-bold text-orange-600 dark:text-orange-400">
              {formatDuration(stats.duration)}
            </div>
            <div className="text-xs text-muted-foreground">
              {t('agent.stats.duration', 'Duration')}
            </div>
          </div>
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
