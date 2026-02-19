import { useState, useEffect, useMemo, useRef, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Skeleton } from '../ui/skeleton';
import { taskApi, AgentTask, TaskResult } from '../../api/agent';
import { ExecutionStatsPanel } from './ExecutionStatsPanel';
import { ToolCallTimeline } from './ToolCallTimeline';
import {
  parseToolCalls,
  cleanMessageText,
  calculateStats,
  ToolCall,
} from './TaskResultParser';

function extractText(content: unknown): string {
  if (typeof content === 'string') return content;
  return (content as { text?: string })?.text || JSON.stringify(content);
}

// Merge consecutive message results into single entries
function mergeResults(results: TaskResult[]): TaskResult[] {
  if (results.length === 0) return [];

  const merged: TaskResult[] = [];
  let currentMessage: TaskResult | null = null;
  let messageTexts: string[] = [];

  for (const result of results) {
    if (result.result_type === 'message') {
      const content = extractText(result.content);

      if (!currentMessage) {
        currentMessage = { ...result };
        messageTexts = [content];
      } else {
        messageTexts.push(content);
      }
    } else {
      // Flush accumulated messages
      if (currentMessage && messageTexts.length > 0) {
        merged.push({
          ...currentMessage,
          content: messageTexts.join(''),
          result_type: 'message'
        });
        currentMessage = null;
        messageTexts = [];
      }
      merged.push(result);
    }
  }

  // Flush remaining messages
  if (currentMessage && messageTexts.length > 0) {
    merged.push({
      ...currentMessage,
      content: messageTexts.join(''),
      result_type: 'message'
    });
  }

  return merged;
}

const STATUS_VARIANTS: Record<string, 'default' | 'secondary' | 'destructive' | 'outline'> = {
  pending: 'outline',
  approved: 'secondary',
  rejected: 'destructive',
  running: 'default',
  completed: 'default',
  failed: 'destructive',
  cancelled: 'secondary',
};

interface Props {
  task: AgentTask | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAction: () => void;
}

export function TaskDetailDialog({ task, open, onOpenChange, onAction }: Props) {
  const { t } = useTranslation();
  const [results, setResults] = useState<TaskResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);
  const [toolFilter, setToolFilter] = useState<string | null>(null);
  const [streamMessages, setStreamMessages] = useState<{ type: string; content: string; timestamp: number }[]>([]);
  const eventSourceRef = useRef<EventSource | null>(null);

  // Parse tool calls, merge results, and calculate stats in one pass
  const { mergedResults, toolCalls, stats, cleanedContent } = useMemo(() => {
    const merged = mergeResults(results);
    const allToolCalls: ToolCall[] = [];
    const parsedResults: { type: 'message' | 'tool_call' | 'error'; content: string; toolCalls: ToolCall[]; timestamp: Date }[] = [];
    let fullContent = '';

    for (const result of merged) {
      const text = result.result_type === 'message' ? extractText(result.content) : '';
      const calls = parseToolCalls(text);
      calls.forEach(c => { c.timestamp = new Date(result.created_at); });
      allToolCalls.push(...calls);
      fullContent += text;
      parsedResults.push({
        type: result.result_type as 'message' | 'tool_call' | 'error',
        content: text,
        toolCalls: calls,
        timestamp: new Date(result.created_at),
      });
    }

    return {
      mergedResults: merged,
      toolCalls: allToolCalls,
      stats: calculateStats(parsedResults),
      cleanedContent: cleanMessageText(fullContent),
    };
  }, [results]);

  const loadResults = useCallback(async () => {
    if (!task) return;
    setLoading(true);
    try {
      const data = await taskApi.getTaskResults(task.id);
      setResults(data);
    } catch (error) {
      console.error('Failed to load results:', error);
    } finally {
      setLoading(false);
    }
  }, [task]);

  useEffect(() => {
    if (task && open) {
      loadResults();
      setStreamMessages([]);
    }
  }, [task, open, loadResults]);

  // SSE streaming for running tasks
  useEffect(() => {
    if (!task || !open) return;
    if (task.status !== 'running' && task.status !== 'approved') return;

    const es = taskApi.streamTaskResults(task.id);
    eventSourceRef.current = es;

    const handleEvent = (type: string) => (e: MessageEvent) => {
      try {
        const data = JSON.parse(e.data);
        if (type === 'done') {
          es.close();
          eventSourceRef.current = null;
          loadResults();
          return;
        }
        if (type === 'status') {
          loadResults();
          return;
        }
        setStreamMessages(prev => [...prev, {
          type,
          content: data.content || data.text || data.name || JSON.stringify(data),
          timestamp: Date.now(),
        }]);
      } catch { /* ignore */ }
    };

    es.addEventListener('text', handleEvent('text'));
    es.addEventListener('thinking', handleEvent('thinking'));
    es.addEventListener('toolcall', handleEvent('toolcall'));
    es.addEventListener('toolresult', handleEvent('toolresult'));
    es.addEventListener('status', handleEvent('status'));
    es.addEventListener('done', handleEvent('done'));

    return () => {
      es.close();
      eventSourceRef.current = null;
    };
  }, [task?.id, task?.status, open, loadResults]);

  const handleAction = useCallback(async (action: (id: string) => Promise<unknown>) => {
    if (!task) return;
    setActionLoading(true);
    try {
      await action(task.id);
      onAction();
    } finally {
      setActionLoading(false);
    }
  }, [task, onAction]);

  if (!task) return null;

  const content = task.content as { messages?: { role: string; content: string }[] };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[700px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-3">
            <span>{t('agent.task.detail', 'Task Detail')}</span>
            <Badge variant={STATUS_VARIANTS[task.status] || 'outline'}>{t(`agent.status.${task.status}`)}</Badge>
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {/* Task Info */}
          <div className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <span className="text-muted-foreground">{t('agent.task.id', 'Task ID')}:</span>
              <span className="ml-2 font-mono">{task.id.slice(0, 8)}...</span>
            </div>
            <div>
              <span className="text-muted-foreground">{t('agent.task.type', 'Type')}:</span>
              <span className="ml-2">{task.task_type}</span>
            </div>
            <div>
              <span className="text-muted-foreground">{t('agent.task.submittedAt', 'Submitted')}:</span>
              <span className="ml-2">{new Date(task.submitted_at).toLocaleString()}</span>
            </div>
            {task.approved_at && (
              <div>
                <span className="text-muted-foreground">{t('agent.task.approvedAt', 'Approved')}:</span>
                <span className="ml-2">{new Date(task.approved_at).toLocaleString()}</span>
              </div>
            )}
          </div>

          {/* Task Content */}
          <div className="space-y-2">
            <h4 className="font-medium">{t('agent.task.content', 'Task Content')}</h4>
            <div className="bg-muted p-3 rounded-lg">
              {content?.messages?.map((msg, i) => (
                <div key={i} className="mb-2">
                  <span className="text-xs text-muted-foreground uppercase">{msg.role}:</span>
                  <p className="whitespace-pre-wrap">{msg.content}</p>
                </div>
              ))}
            </div>
          </div>

          {/* Results */}
          <div className="space-y-4">
            <h4 className="font-medium">{t('agent.task.results', 'Results')}</h4>
            {loading ? (
              <Skeleton className="h-20 w-full" />
            ) : mergedResults.length === 0 ? (
              <p className="text-muted-foreground text-sm">
                {t('agent.task.noResults', 'No results yet')}
              </p>
            ) : (
              <div className="space-y-4">
                {/* Statistics Panel */}
                {stats.totalToolCalls > 0 && (
                  <ExecutionStatsPanel stats={stats} />
                )}

                {/* Tool Call Timeline */}
                {toolCalls.length > 0 && (
                  <ToolCallTimeline
                    toolCalls={toolCalls}
                    filter={toolFilter}
                    onFilterChange={setToolFilter}
                  />
                )}

                {/* Final Result */}
                {cleanedContent && (
                  <div className="space-y-2">
                    <div className="flex items-center gap-2">
                      <span>📝</span>
                      <h5 className="font-medium text-sm">
                        {t('agent.task.finalResult', 'Final Result')}
                      </h5>
                    </div>
                    <div className="bg-muted p-3 rounded-lg">
                      <pre className="text-sm whitespace-pre-wrap overflow-x-auto">
                        {cleanedContent}
                      </pre>
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* Live Stream */}
          {streamMessages.length > 0 && (
            <div className="space-y-2">
              <h4 className="font-medium flex items-center gap-2">
                <span className="w-2 h-2 bg-green-500 rounded-full animate-pulse" />
                {t('agent.task.liveStream', 'Live Output')}
              </h4>
              <div className="bg-muted p-3 rounded-lg max-h-60 overflow-y-auto space-y-1">
                {streamMessages.map((msg, i) => (
                  <div key={i} className="text-sm">
                    {msg.type === 'toolcall' && (
                      <span className="text-blue-500 font-mono">🔧 {msg.content}</span>
                    )}
                    {msg.type === 'toolresult' && (
                      <span className="text-muted-foreground font-mono text-xs">  ↳ {msg.content.slice(0, 200)}</span>
                    )}
                    {msg.type === 'text' && (
                      <span className="whitespace-pre-wrap">{msg.content}</span>
                    )}
                    {msg.type === 'thinking' && (
                      <span className="text-muted-foreground italic">{msg.content}</span>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Actions */}
          {task.status === 'pending' && (
            <div className="flex gap-2 pt-4 border-t">
              <Button onClick={() => handleAction(taskApi.approveTask)} disabled={actionLoading}>
                {t('agent.actions.approve')}
              </Button>
              <Button variant="outline" onClick={() => handleAction(taskApi.rejectTask)} disabled={actionLoading}>
                {t('agent.actions.reject')}
              </Button>
            </div>
          )}
          {(task.status === 'approved' || task.status === 'running') && (
            <div className="flex gap-2 pt-4 border-t">
              <Button variant="destructive" onClick={() => handleAction(taskApi.cancelTask)} disabled={actionLoading}>
                {t('agent.actions.cancel')}
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
