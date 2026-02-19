/**
 * @deprecated This component is replaced by the ChatPage + ChatConversation (Phase 1 Chat Track).
 * It remains for backward compatibility but should not be used in new code.
 * Use `/teams/:teamId/chat` route instead.
 */
import { useState, useEffect, useRef, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Textarea } from '../ui/textarea';
import { Skeleton } from '../ui/skeleton';
import { taskApi, TeamAgent } from '../../api/agent';
import { Brain, Wrench, ChevronDown, ChevronUp, Zap } from 'lucide-react';

interface ToolCallInfo {
  id: string;
  name: string;
  success?: boolean;
  result?: string;
}

interface Message {
  role: 'user' | 'assistant';
  content: string;
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  sessionId?: string;
  timestamp: Date;
  taskId?: string;
  status?: string;
  isStreaming?: boolean;
}

interface Props {
  agent: TeamAgent | null;
  teamId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function ChatDialog({ agent, teamId, open, onOpenChange }: Props) {
  const { t } = useTranslation();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [streamingTaskId, setStreamingTaskId] = useState<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [expandedThinking, setExpandedThinking] = useState<Set<number>>(new Set());
  const [expandedTools, setExpandedTools] = useState<Set<number>>(new Set());
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const accumulatedContentRef = useRef<string>('');
  const sessionIdRef = useRef<string | null>(null);

  // Keep sessionIdRef in sync with state to avoid stale closures
  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  // Scroll to bottom when messages change
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  // Cleanup EventSource on unmount or dialog close
  useEffect(() => {
    return () => {
      if (eventSourceRef.current) {
        eventSourceRef.current.close();
        eventSourceRef.current = null;
      }
    };
  }, []);

  // Start SSE streaming for a task
  const startStreaming = useCallback((taskId: string) => {
    // Close existing connection
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
    }

    accumulatedContentRef.current = '';
    setStreamingTaskId(taskId);

    const eventSource = new EventSource(`/api/team/agent/tasks/${taskId}/stream`);
    eventSourceRef.current = eventSource;

    // Handle status events
    eventSource.addEventListener('status', (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.status === 'running') {
          setMessages(prev => prev.map(msg =>
            msg.taskId === taskId ? { ...msg, status: 'running' } : msg
          ));
        }
      } catch (e) {
        console.error('SSE status parse error:', e);
      }
    });

    // Handle text events (streaming content)
    eventSource.addEventListener('text', (event) => {
      try {
        const data = JSON.parse(event.data);
        const content = data.content;
        if (content) {
          accumulatedContentRef.current += content;
          setMessages(prev => prev.map(msg => {
            if (msg.taskId === taskId) {
              return { ...msg, content: accumulatedContentRef.current, status: 'running', isStreaming: true };
            }
            return msg;
          }));
        }
      } catch (e) {
        console.error('SSE text parse error:', e);
      }
    });

    // Handle thinking events (extended thinking from Claude)
    eventSource.addEventListener('thinking', (event) => {
      try {
        const data = JSON.parse(event.data);
        const content = data.content;
        if (content) {
          setMessages(prev => prev.map(msg => {
            if (msg.taskId === taskId) {
              const existing = msg.thinking || '';
              return { ...msg, thinking: existing + content, status: 'running', isStreaming: true };
            }
            return msg;
          }));
        }
      } catch (e) {
        console.error('SSE thinking parse error:', e);
      }
    });

    // Handle tool call events
    eventSource.addEventListener('toolcall', (event) => {
      try {
        const data = JSON.parse(event.data);
        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId) {
            const calls = msg.toolCalls || [];
            return { ...msg, toolCalls: [...calls, { id: data.id, name: data.name }], isStreaming: true };
          }
          return msg;
        }));
      } catch (e) {
        console.error('SSE toolcall parse error:', e);
      }
    });

    // Handle tool result events
    eventSource.addEventListener('toolresult', (event) => {
      try {
        const data = JSON.parse(event.data);
        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId && msg.toolCalls) {
            const calls = msg.toolCalls.map(tc =>
              tc.id === data.id ? { ...tc, success: data.success, result: data.content } : tc
            );
            return { ...msg, toolCalls: calls };
          }
          return msg;
        }));
      } catch (e) {
        console.error('SSE toolresult parse error:', e);
      }
    });

    // Handle turn progress events
    eventSource.addEventListener('turn', (event) => {
      try {
        const data = JSON.parse(event.data);
        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId) {
            return { ...msg, turn: { current: data.current, max: data.max } };
          }
          return msg;
        }));
      } catch (e) {
        console.error('SSE turn parse error:', e);
      }
    });

    // Handle compaction events
    eventSource.addEventListener('compaction', (event) => {
      try {
        const data = JSON.parse(event.data);
        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId) {
            return { ...msg, compaction: { strategy: data.strategy, before: data.before_tokens, after: data.after_tokens } };
          }
          return msg;
        }));
      } catch (e) {
        console.error('SSE compaction parse error:', e);
      }
    });

    // Handle session ID events — persist to component state for cross-message continuity
    eventSource.addEventListener('session_id', (event) => {
      try {
        const data = JSON.parse(event.data);
        if (data.session_id) {
          sessionIdRef.current = data.session_id;
          setSessionId(data.session_id);
        }
        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId) {
            return { ...msg, sessionId: data.session_id };
          }
          return msg;
        }));
      } catch (e) {
        console.error('SSE session_id parse error:', e);
      }
    });

    // Handle done events (task completed)
    eventSource.addEventListener('done', (event) => {
      try {
        const data = JSON.parse(event.data);
        const status = data.status || 'completed';
        const error = data.error;

        eventSource.close();
        eventSourceRef.current = null;
        setStreamingTaskId(null);

        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId) {
            const finalContent = status === 'failed'
              ? (error || 'Task failed')
              : (accumulatedContentRef.current || 'No response');
            return { ...msg, status, content: finalContent, isStreaming: false };
          }
          return msg;
        }));
      } catch (e) {
        console.error('SSE done parse error:', e);
      }
    });

    eventSource.onerror = (e) => {
      // EventSource triggers onerror when connection closes (even normally)
      console.log('SSE connection error or closed:', e);
      if (eventSourceRef.current) {
        eventSource.close();
        eventSourceRef.current = null;
        setStreamingTaskId(null);

        // If we have accumulated content, show it; otherwise show error
        setMessages(prev => prev.map(msg => {
          if (msg.taskId === taskId && msg.isStreaming) {
            const finalContent = accumulatedContentRef.current ||
              t('agent.chat.connectionLost', 'Connection lost. Check task results for details.');
            return { ...msg, isStreaming: false, content: finalContent };
          }
          return msg;
        }));
      }
    };
  }, []);

  const handleSend = async () => {
    if (!input.trim() || !agent || sending) return;

    const userMessage: Message = {
      role: 'user',
      content: input.trim(),
      timestamp: new Date(),
    };

    setMessages(prev => [...prev, userMessage]);
    setInput('');
    setSending(true);

    try {
      // Submit task with session_id for cross-message continuity
      const task = await taskApi.submitTask({
        team_id: teamId,
        agent_id: agent.id,
        task_type: 'chat',
        content: {
          messages: [{ role: 'user', content: userMessage.content }],
          ...(sessionIdRef.current ? { session_id: sessionIdRef.current } : {}),
        },
      });

      // Add placeholder for assistant response
      const assistantMessage: Message = {
        role: 'assistant',
        content: task.status === 'approved'
          ? t('agent.chat.thinking', 'Thinking...')
          : t('agent.chat.waiting', 'Waiting for approval...'),
        timestamp: new Date(),
        taskId: task.id,
        status: task.status === 'approved' ? 'running' : 'pending',
        isStreaming: task.status === 'approved',
      };
      setMessages(prev => [...prev, assistantMessage]);

      if (task.status === 'approved') {
        // Auto-approved by backend, start streaming directly
        startStreaming(task.id);
      } else {
        // Try to approve (only admins can approve)
        try {
          await taskApi.approveTask(task.id);
          // Approval succeeded, update status and start streaming
          setMessages(prev => prev.map(msg =>
            msg.taskId === task.id
              ? { ...msg, status: 'running', content: t('agent.chat.thinking', 'Thinking...'), isStreaming: true }
              : msg
          ));
          startStreaming(task.id);
        } catch (approveError) {
          // Approval failed (likely 403 Forbidden for non-admins)
          // Start streaming to check if admin approves later
          console.log('Auto-approve failed, waiting for admin approval');
          startStreaming(task.id);
        }
      }
    } catch (error) {
      console.error('Send error:', error);
      const errorMessage = error instanceof Error ? error.message : String(error);
      setMessages(prev => [...prev, {
        role: 'assistant',
        content: `Error: ${errorMessage}`,
        timestamp: new Date(),
      }]);
    } finally {
      setSending(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  if (!agent) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px] h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>
            {t('agent.chat.title', 'Chat with')} {agent.name}
          </DialogTitle>
        </DialogHeader>

        {/* Messages */}
        <div className="flex-1 overflow-y-auto space-y-4 py-4">
          {messages.length === 0 ? (
            <div className="text-center text-muted-foreground py-8">
              {t('agent.chat.empty', 'Start a conversation with the agent')}
            </div>
          ) : (
            messages.map((msg, i) => (
              <div
                key={i}
                className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
              >
                <div
                  className={`max-w-[85%] rounded-lg px-4 py-2 ${
                    msg.role === 'user'
                      ? 'bg-primary text-primary-foreground'
                      : 'bg-muted'
                  }`}
                >
                  {/* Turn progress indicator */}
                  {msg.turn && msg.isStreaming && (
                    <div className="text-xs text-muted-foreground mb-1 flex items-center gap-1">
                      <Zap className="h-3 w-3" />
                      {t('agent.chat.turn', 'Turn {{current}}/{{max}}', { current: msg.turn.current, max: msg.turn.max })}
                    </div>
                  )}

                  {/* Thinking section (collapsible) */}
                  {msg.thinking && (
                    <div className="mb-2 border-l-2 border-purple-400/50 pl-2">
                      <button
                        type="button"
                        className="flex items-center gap-1 text-xs text-purple-500 hover:text-purple-700 mb-1"
                        onClick={() => {
                          const next = new Set(expandedThinking);
                          next.has(i) ? next.delete(i) : next.add(i);
                          setExpandedThinking(next);
                        }}
                      >
                        <Brain className="h-3 w-3" />
                        {t('agent.chat.thinkingLabel', 'Thinking')}
                        {expandedThinking.has(i) ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                      </button>
                      {expandedThinking.has(i) && (
                        <p className="text-xs text-muted-foreground whitespace-pre-wrap max-h-40 overflow-y-auto">
                          {msg.thinking}
                        </p>
                      )}
                    </div>
                  )}

                  {/* Tool calls section (collapsible) */}
                  {msg.toolCalls && msg.toolCalls.length > 0 && (
                    <div className="mb-2 border-l-2 border-blue-400/50 pl-2">
                      <button
                        type="button"
                        className="flex items-center gap-1 text-xs text-blue-500 hover:text-blue-700 mb-1"
                        onClick={() => {
                          const next = new Set(expandedTools);
                          next.has(i) ? next.delete(i) : next.add(i);
                          setExpandedTools(next);
                        }}
                      >
                        <Wrench className="h-3 w-3" />
                        {t('agent.chat.toolCallsLabel', '{{count}} tool calls', { count: msg.toolCalls.length })}
                        {expandedTools.has(i) ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />}
                      </button>
                      {expandedTools.has(i) && (
                        <div className="space-y-1 max-h-40 overflow-y-auto">
                          {msg.toolCalls.map((tc) => (
                            <div key={tc.id} className="text-xs flex items-center gap-1">
                              <span className={`inline-block w-1.5 h-1.5 rounded-full ${
                                tc.success === undefined ? 'bg-yellow-400 animate-pulse' :
                                tc.success ? 'bg-green-500' : 'bg-red-500'
                              }`} />
                              <span className="font-mono">{tc.name}</span>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  )}

                  {/* Main content */}
                  {msg.status === 'running' || msg.isStreaming ? (
                    <div className="flex items-start gap-2">
                      {msg.isStreaming && <Skeleton className="h-4 w-4 rounded-full animate-pulse mt-1 shrink-0" />}
                      <span className="whitespace-pre-wrap">{msg.content}</span>
                    </div>
                  ) : (
                    <p className="whitespace-pre-wrap">{msg.content}</p>
                  )}

                  {/* Compaction notification */}
                  {msg.compaction && (
                    <div className="text-xs text-amber-600 mt-1 flex items-center gap-1">
                      <Zap className="h-3 w-3" />
                      {t('agent.chat.compacted', 'Context compacted: {{before}} → {{after}} tokens', {
                        before: msg.compaction.before,
                        after: msg.compaction.after,
                      })}
                    </div>
                  )}

                  <span className="text-xs opacity-70 mt-1 block">
                    {msg.timestamp.toLocaleTimeString()}
                  </span>
                </div>
              </div>
            ))
          )}
          <div ref={messagesEndRef} />
        </div>

        {/* Input */}
        <div className="border-t pt-4">
          <div className="flex gap-2">
            <Textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder={t('agent.chat.placeholder', 'Type a message...')}
              className="min-h-[60px] resize-none"
              disabled={sending || !!streamingTaskId}
            />
            <Button
              onClick={handleSend}
              disabled={!input.trim() || sending || !!streamingTaskId}
              className="self-end"
            >
              {sending ? '...' : t('agent.chat.send', 'Send')}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
