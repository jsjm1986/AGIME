import { useState, useEffect, useRef, useCallback } from 'react';
import { Loader2, Paperclip, X, Bot, ChevronDown, ChevronRight, Zap, Puzzle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useAuth } from '../../contexts/AuthContext';
import { chatApi, type CreateSessionOptions } from '../../api/chat';
import { ChatMessageBubble } from './ChatMessageBubble';
import { ChatInput } from './ChatInput';
import { DocumentPicker } from '../documents/DocumentPicker';
import type { DocumentSummary } from '../../api/documents';
import type { TeamAgent } from '../../api/agent';
import type { Message } from './ChatMessageBubble';

export interface ChatRuntimeEvent {
  kind: 'status' | 'turn' | 'toolcall' | 'toolresult' | 'compaction' | 'workspace_changed' | 'done' | 'connection' | 'goal';
  text: string;
  ts: number;
  detail?: Record<string, unknown>;
}

interface ChatConversationProps {
  sessionId: string | null;
  agentId: string;
  agentName: string;
  agent?: TeamAgent | null;
  teamId?: string;
  initialAttachedDocIds?: string[];
  /** Optional custom session factory for specialized flows (e.g. portal lab coding sessions) */
  createSession?: () => Promise<string>;
  createSessionOptions?: CreateSessionOptions;
  onSessionCreated?: (sessionId: string) => void;
  /** Called when a tool result is received during streaming */
  onToolResult?: (toolName: string, result: string, success: boolean) => void;
  /** Called when processing state changes */
  onProcessingChange?: (processing: boolean) => void;
  /** Called when runtime stream event arrives (for timeline/observability UI) */
  onRuntimeEvent?: (event: ChatRuntimeEvent) => void;
  /** Optional error callback for surfacing failures in parent UI */
  onError?: (message: string) => void;
}

export function ChatConversation({
  sessionId,
  agentId,
  agentName,
  agent,
  teamId,
  initialAttachedDocIds,
  createSession,
  createSessionOptions,
  onSessionCreated,
  onToolResult,
  onProcessingChange,
  onRuntimeEvent,
  onError,
}: ChatConversationProps) {
  const { t } = useTranslation();
  const { user } = useAuth();
  const [messages, setMessages] = useState<Message[]>([]);
  const [isProcessing, setIsProcessing] = useState(false);
  const [liveStatus, setLiveStatus] = useState('');
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [loading, setLoading] = useState(false);
  const [attachedDocs, setAttachedDocs] = useState<DocumentSummary[]>([]);
  const [showDocPicker, setShowDocPicker] = useState(false);
  const [pendingDocIds, setPendingDocIds] = useState<string[]>(initialAttachedDocIds || []);
  const [showCapabilities, setShowCapabilities] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const eventSourceRef = useRef<EventSource | null>(null);
  const currentSessionRef = useRef<string | null>(sessionId);
  const justCreatedRef = useRef(false);
  const toolCallNamesRef = useRef<Map<string, string>>(new Map());
  const reconnectAttemptsRef = useRef(0);
  const reconnectTimerRef = useRef<number | null>(null);
  const processingStartedAtRef = useRef<number | null>(null);
  const lastEventIdRef = useRef<number | null>(null);
  const lastRuntimeEventAtRef = useRef<number>(0);
  const isProcessingRef = useRef(false);
  const sessionSyncInFlightRef = useRef(false);

  // Keep ref in sync
  useEffect(() => {
    currentSessionRef.current = sessionId;
    lastEventIdRef.current = null;
  }, [sessionId]);

  // Surface processing state to parent and maintain elapsed timer anchors
  useEffect(() => {
    isProcessingRef.current = isProcessing;
    onProcessingChange?.(isProcessing);
    if (isProcessing) {
      if (!processingStartedAtRef.current) {
        processingStartedAtRef.current = Date.now();
      }
    } else {
      processingStartedAtRef.current = null;
      lastRuntimeEventAtRef.current = 0;
      setElapsedSeconds(0);
      reconnectAttemptsRef.current = 0;
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
    }
  }, [isProcessing, onProcessingChange]);

  // Update elapsed seconds while processing
  useEffect(() => {
    if (!isProcessing) return;
    const timer = window.setInterval(() => {
      if (!processingStartedAtRef.current) return;
      setElapsedSeconds(Math.max(0, Math.floor((Date.now() - processingStartedAtRef.current) / 1000)));
    }, 1000);
    return () => window.clearInterval(timer);
  }, [isProcessing]);

  // Load session messages
  useEffect(() => {
    if (!sessionId) {
      setMessages([]);
      return;
    }
    // Skip loadSession if we just created this session in handleSend
    // (loadSession would overwrite the optimistic messages with empty DB content)
    if (justCreatedRef.current) {
      justCreatedRef.current = false;
      return;
    }
    loadSession(sessionId);
  }, [sessionId]);

  // M17: Auto-scroll only when user is near the bottom
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const threshold = 150;
    const isNearBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight < threshold;
    if (isNearBottom) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      eventSourceRef.current?.close();
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
      }
    };
  }, []);

  const loadSession = async (sid: string) => {
    setLoading(true);
    try {
      const detail = await chatApi.getSession(sid);
      const parsed = parseMessages(detail.messages_json);
      setIsProcessing(detail.is_processing);
      if (detail.is_processing) {
        // Ensure a streaming assistant placeholder exists so SSE events
        // have a target message to append to after component remount.
        const lastMsg = parsed[parsed.length - 1];
        if (!lastMsg || lastMsg.role !== 'assistant') {
          parsed.push({
            id: `resume-${Date.now()}`,
            role: 'assistant',
            content: '',
            isStreaming: true,
            timestamp: new Date(),
          });
        } else if (!lastMsg.isStreaming) {
          // DB has a saved assistant message but streaming is still active
          parsed[parsed.length - 1] = { ...lastMsg, isStreaming: true };
        }
        setMessages(parsed);
        const resumeLabel = t('chat.resumeProcessing', 'Session is running, reconnecting stream...');
        setLiveStatus(resumeLabel);
        emitRuntimeEvent('connection', resumeLabel);
        connectStream(sid);
      } else {
        setMessages(parsed);
      }
    } catch (e) {
      console.error('Failed to load session:', e);
    } finally {
      setLoading(false);
    }
  };

  const parseMessages = (json: string): Message[] => {
    const normalizeRole = (rawRole: unknown): Message['role'] | null => {
      if (typeof rawRole === 'string') {
        const lowered = rawRole.toLowerCase();
        if (lowered === 'user') return 'user';
        if (lowered === 'assistant') return 'assistant';
        return null;
      }
      if (rawRole && typeof rawRole === 'object') {
        const obj = rawRole as Record<string, unknown>;
        const roleField = obj.role;
        if (typeof roleField === 'string') {
          const lowered = roleField.toLowerCase();
          if (lowered === 'user') return 'user';
          if (lowered === 'assistant') return 'assistant';
        }
        if (obj.User !== undefined || obj.user !== undefined) return 'user';
        if (obj.Assistant !== undefined || obj.assistant !== undefined) return 'assistant';
      }
      return null;
    };

    try {
      const raw = JSON.parse(json);
      if (!Array.isArray(raw)) return [];
      const parsed: Message[] = [];
      for (let i = 0; i < raw.length; i += 1) {
        const m = raw[i];
        const role = normalizeRole(m?.role);
        if (!role) continue;
        const meta = m?.metadata || {};
        const userVisible = (meta?.user_visible ?? meta?.userVisible) !== false;
        if (!userVisible) continue;

        let text = '';
        let thinking = '';
        const toolCalls: Array<{ name: string; id: string }> = [];
        if (typeof m.content === 'string') {
          text = m.content;
        } else if (Array.isArray(m.content)) {
          for (const c of m.content) {
            const cType = String(c?.type || '').toLowerCase();
            if (cType === 'text' || (!cType && c?.text)) {
              text += c?.text || '';
              continue;
            }
            if (cType === 'thinking' && c?.thinking) {
              thinking += c.thinking;
              continue;
            }
            if (
              (cType === 'toolrequest' || cType === 'tool_request' || cType === 'tool_use' || cType === 'tooluse') &&
              (c?.toolCall || c?.tool_call || c)
            ) {
              const tc = c.toolCall || c.tool_call || c;
              toolCalls.push({
                id: c.id || tc.id || `hist-tool-${i}-${toolCalls.length}`,
                name: tc.name || 'tool',
              });
            }
            if (cType === 'systemnotification' && typeof c?.msg === 'string') {
              text += c.msg;
            }
          }
        } else if (m.content && typeof m.content === 'object') {
          const c = m.content as Record<string, unknown>;
          if (typeof c.text === 'string') {
            text += c.text;
          } else if (typeof c.msg === 'string') {
            text += c.msg;
          }
        }

        if (!(text.trim().length > 0 || !!thinking || toolCalls.length > 0)) continue;
        const createdRaw = Number(m?.created ?? m?.timestamp ?? 0);
        const createdMs = Number.isFinite(createdRaw)
          ? (createdRaw > 10_000_000_000 ? createdRaw : createdRaw * 1000)
          : 0;
        parsed.push({
          id: `hist-${i}`,
          role,
          content: text,
          thinking: thinking || undefined,
          toolCalls: toolCalls.length > 0 ? toolCalls : undefined,
          timestamp: createdMs > 0 ? new Date(createdMs) : new Date(),
        });
      }
      return parsed;
    } catch {
      return [];
    }
  };

  const emitRuntimeEvent = useCallback(
    (kind: ChatRuntimeEvent['kind'], text: string, detail?: Record<string, unknown>) => {
      lastRuntimeEventAtRef.current = Date.now();
      onRuntimeEvent?.({
        kind,
        text,
        ts: Date.now(),
        detail,
      });
    },
    [onRuntimeEvent]
  );

  const handleSend = useCallback(async (content: string) => {
    // M19: Prevent double-click race
    if (isProcessing) return;

    let sid = currentSessionRef.current;

    // Create session if needed
    if (!sid) {
      try {
        if (createSession) {
          sid = await createSession();
        } else {
          const docIds = pendingDocIds.length > 0 ? pendingDocIds : undefined;
          const res = await chatApi.createSession(agentId, docIds, createSessionOptions);
          sid = res.session_id;
        }
        currentSessionRef.current = sid;
        lastEventIdRef.current = null;
        setPendingDocIds([]);
        justCreatedRef.current = true;
        onSessionCreated?.(sid);
      } catch (e) {
        console.error('Failed to create session:', e);
        const msg = t('chat.sessionCreateFailed', 'Failed to start session');
        setLiveStatus(msg);
        emitRuntimeEvent('done', msg);
        onError?.(msg);
        return;
      }
    }

    // Attach pending documents to existing session before sending
    if (pendingDocIds.length > 0) {
      try {
        await chatApi.attachDocuments(sid, pendingDocIds);
        setPendingDocIds([]);
        setAttachedDocs([]);
      } catch (e) {
        console.error('Failed to attach documents:', e);
      }
    }

    // M16: Use stable IDs for React keys
    const now = Date.now();
    const userMsgId = `msg-${now}-user`;
    const assistantMsgId = `msg-${now}-assistant`;

    // Add user message and placeholder assistant message in a single update
    setMessages(prev => [...prev,
      { id: userMsgId, role: 'user' as const, content, timestamp: new Date() },
      { id: assistantMsgId, role: 'assistant' as const, content: '', isStreaming: true, timestamp: new Date() },
    ]);

    setLiveStatus(t('chat.requestSent', 'Request sent, waiting for agent...'));
    emitRuntimeEvent('status', t('chat.requestSent', 'Request sent, waiting for agent...'));
    setIsProcessing(true);
    isProcessingRef.current = true;
    processingStartedAtRef.current = Date.now();

    try {
      await chatApi.sendMessage(sid, content);
      connectStream(sid);
    } catch (e) {
      console.error('Failed to send message:', e);
      setIsProcessing(false);
      const msg = t('chat.sendFailed', 'Request failed');
      setLiveStatus(msg);
      emitRuntimeEvent('done', msg);
      onError?.(msg);
      // Remove placeholder
      setMessages(prev => prev.slice(0, -1));
    }
  }, [
    agentId,
    createSession,
    createSessionOptions,
    emitRuntimeEvent,
    isProcessing,
    onSessionCreated,
    onError,
    pendingDocIds,
    t,
  ]);

  const formatStatusLabel = useCallback((raw: string) => {
    const status = (raw || '').toLowerCase();
    if (!status) return t('chat.processing', 'Processing...');
    if (status === 'running') return t('chat.processing', 'Processing...');
    if (status.includes('llm')) return t('chat.statusLlm', 'Calling model...');
    if (status.includes('portal_tool_retry')) return t('chat.statusPortalRetry', 'Portal coding mode: forcing tool execution...');
    if (status.includes('tool')) return t('chat.statusTool', 'Executing tools...');
    if (status.includes('compaction')) return t('chat.statusCompaction', 'Compacting context...');
    return raw;
  }, [t]);

  useEffect(() => {
    if (!isProcessing) return;
    if (!lastRuntimeEventAtRef.current) {
      lastRuntimeEventAtRef.current = Date.now();
    }
    const timer = window.setInterval(() => {
      const now = Date.now();
      if (now - lastRuntimeEventAtRef.current < 15000) {
        return;
      }
      const heartbeat = t('chat.statusHeartbeat', 'Agent is still running...');
      setLiveStatus(heartbeat);
      emitRuntimeEvent('status', heartbeat, { source: 'heartbeat' });
    }, 5000);

    return () => window.clearInterval(timer);
  }, [emitRuntimeEvent, isProcessing, t]);

  const connectStream = (sid: string, isReconnect = false) => {
    // M20: Close any existing EventSource before opening new one
    eventSourceRef.current?.close();
    eventSourceRef.current = null;
    if (!isReconnect) {
      reconnectAttemptsRef.current = 0;
    }

    const es = chatApi.streamChat(sid, lastEventIdRef.current);
    eventSourceRef.current = es;
    const connectedLabel =
      isReconnect
        ? t('chat.reconnected', 'Reconnected, syncing...')
        : t('chat.streamConnected', 'Connected, waiting for updates...');
    setLiveStatus(connectedLabel);
    emitRuntimeEvent('connection', connectedLabel);
    es.onopen = () => {
      reconnectAttemptsRef.current = 0;
      const openedLabel = t('chat.processing', 'Processing...');
      setLiveStatus(openedLabel);
      emitRuntimeEvent('connection', openedLabel);
    };

    // H6: Wrap all JSON.parse calls in try/catch
    const safeParse = (data: string) => {
      try {
        return JSON.parse(data);
      } catch {
        console.warn('Failed to parse SSE data:', data);
        return null;
      }
    };

    const captureEventId = (evt: Event) => {
      const raw = (evt as MessageEvent).lastEventId;
      const parsed = Number(raw || 0);
      if (Number.isFinite(parsed) && parsed > 0) {
        lastEventIdRef.current = parsed;
      }
    };

    es.addEventListener('text', (e) => {
      // H5: Ignore events for stale sessions
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      updateLastAssistant(msg => ({
        ...msg,
        content: msg.content + data.content,
      }));
    });

    es.addEventListener('thinking', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      updateLastAssistant(msg => ({
        ...msg,
        thinking: (msg.thinking || '') + data.content,
      }));
    });

    es.addEventListener('toolcall', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      if (data.name) {
        const label = t('chat.executingTool', 'Executing tool: {{name}}', { name: data.name });
        setLiveStatus(label);
        emitRuntimeEvent('toolcall', label, { id: data.id, name: data.name });
      }
      // Track tool call id -> name for onToolResult callback
      if (data.id && data.name) {
        toolCallNamesRef.current.set(data.id, data.name);
      }
      updateLastAssistant(msg => ({
        ...msg,
        toolCalls: [...(msg.toolCalls || []), { name: data.name, id: data.id }],
      }));
    });

    es.addEventListener('toolresult', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const toolName = toolCallNamesRef.current.get(data.id) || data.name || '';
      const durationMs = Number(data.duration_ms ?? data.durationMs ?? 0);
      if (data.id) {
        toolCallNamesRef.current.delete(data.id);
      }
      const resultLabel =
        data.success === false
          ? t('chat.toolFailedBy', '{{name}} failed', { name: toolName || t('chat.toolGeneric', 'Tool') })
          : t('chat.toolDoneBy', '{{name}} completed', { name: toolName || t('chat.toolGeneric', 'Tool') });
      const withDuration = durationMs > 0
        ? `${resultLabel} (${t('chat.toolDurationMs', '{{n}}ms', { n: durationMs })})`
        : resultLabel;
      setLiveStatus(withDuration);
      emitRuntimeEvent('toolresult', resultLabel, {
        id: data.id,
        success: data.success !== false,
        toolName,
        durationMs,
        preview: typeof data.content === 'string' ? data.content.slice(0, 200) : '',
      });
      updateLastAssistant(msg => ({
        ...msg,
        toolCalls: (msg.toolCalls || []).map(tc =>
          tc.id === data.id ? { ...tc, result: data.content, success: data.success } : tc
        ),
      }));
      // Notify parent about tool results (e.g. for Portal preview refresh)
      if (toolName) {
        onToolResult?.(toolName, data.content || '', data.success !== false);
      }
    });

    es.addEventListener('turn', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const turnLabel = t('chat.turnProgress', 'Turn {{current}}/{{max}}', { current: data.current, max: data.max });
      setLiveStatus(turnLabel);
      emitRuntimeEvent('turn', turnLabel, { current: data.current, max: data.max });
      updateLastAssistant(msg => ({
        ...msg,
        turn: { current: data.current, max: data.max },
      }));
    });

    es.addEventListener('compaction', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const compactLabel = t('chat.statusCompaction', 'Compacting context...');
      setLiveStatus(compactLabel);
      emitRuntimeEvent('compaction', compactLabel, {
        strategy: data.strategy,
        before: data.before_tokens,
        after: data.after_tokens,
      });
      updateLastAssistant(msg => ({
        ...msg,
        compaction: {
          strategy: data.strategy,
          before: data.before_tokens,
          after: data.after_tokens,
        },
      }));
    });

    es.addEventListener('status', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data?.status) return;
      const label = formatStatusLabel(data.status);
      setLiveStatus(label);
      emitRuntimeEvent('status', label, { rawStatus: data.status });
    });

    es.addEventListener('workspace_changed', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      const toolName = data?.tool_name || data?.toolName || '';
      const label = t('chat.workspaceChangedBy', 'Workspace updated by {{tool}}', {
        tool: toolName || 'tool',
      });
      setLiveStatus(label);
      emitRuntimeEvent('workspace_changed', label, { toolName });
    });

    es.addEventListener('goal_start', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t('chat.goalStart', 'Goal started: {{title}}', {
        title: data.title || data.goal_id || 'goal',
      });
      setLiveStatus(label);
      emitRuntimeEvent('goal', label, data);
    });

    es.addEventListener('goal_complete', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t('chat.goalComplete', 'Goal completed');
      setLiveStatus(label);
      emitRuntimeEvent('goal', label, data);
    });

    es.addEventListener('pivot', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t('chat.goalPivot', 'Plan pivoted');
      setLiveStatus(label);
      emitRuntimeEvent('goal', label, data);
    });

    es.addEventListener('goal_abandoned', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      const label = t('chat.goalAbandoned', 'Goal abandoned');
      setLiveStatus(label);
      emitRuntimeEvent('goal', label, data);
    });

    es.addEventListener('done', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      // Show error in the assistant message if execution failed
      if (data?.error) {
        updateLastAssistant(msg => ({
          ...msg,
          isStreaming: false,
          content: msg.content || `⚠ ${data.error}`,
        }));
      } else {
        updateLastAssistant(msg => {
          const hasReadableContent =
            msg.content.trim().length > 0 ||
            (msg.thinking || '').trim().length > 0 ||
            (msg.toolCalls?.length || 0) > 0;
          return {
            ...msg,
            isStreaming: false,
            content: hasReadableContent
              ? msg.content
              : t('chat.completedNoText', 'Completed. No textual output returned.'),
          };
        });
      }
      setIsProcessing(false);
      isProcessingRef.current = false;
      const doneLabel = data?.error ? t('chat.failed', 'Execution failed') : t('chat.completed', 'Completed');
      setLiveStatus(doneLabel);
      emitRuntimeEvent('done', doneLabel, { error: data?.error || null });
      es.close();
      eventSourceRef.current = null;
    });

    es.onerror = () => {
      if (currentSessionRef.current !== sid) return;
      es.close();
      eventSourceRef.current = null;
      if (!isProcessingRef.current) {
        return;
      }
      const nextAttempt = reconnectAttemptsRef.current + 1;
      reconnectAttemptsRef.current = nextAttempt;
      if (nextAttempt > 6) {
        updateLastAssistant(msg => ({ ...msg, isStreaming: false }));
        setIsProcessing(false);
        isProcessingRef.current = false;
        const disconnectedLabel = t('chat.streamDisconnected', 'Stream disconnected');
        setLiveStatus(disconnectedLabel);
        emitRuntimeEvent('connection', disconnectedLabel);
        return;
      }

      const reconnectingLabel = t('chat.reconnecting', 'Connection lost, reconnecting ({{n}})...', { n: nextAttempt });
      setLiveStatus(reconnectingLabel);
      emitRuntimeEvent('connection', reconnectingLabel, { attempt: nextAttempt });
      const delay = Math.min(1000 * nextAttempt, 5000);
      if (reconnectTimerRef.current) {
        window.clearTimeout(reconnectTimerRef.current);
      }
      reconnectTimerRef.current = window.setTimeout(async () => {
        try {
          const detail = await chatApi.getSession(sid);
          if (currentSessionRef.current !== sid) return;
          if (detail.is_processing) {
            // Sync latest persisted messages before reconnect to reduce visual gaps.
            const parsed = parseMessages(detail.messages_json);
            if (parsed.length > 0) {
              setMessages(parsed);
            }
            connectStream(sid, true);
          } else {
            // Processing already finished while disconnected.
            // Reload canonical session history to avoid missing final output.
            const parsed = parseMessages(detail.messages_json);
            if (parsed.length > 0) {
              setMessages(parsed);
            } else {
              updateLastAssistant(msg => ({ ...msg, isStreaming: false }));
            }
            setIsProcessing(false);
            isProcessingRef.current = false;
            const completedLabel = t('chat.completed', 'Completed');
            setLiveStatus(completedLabel);
            emitRuntimeEvent('done', completedLabel);
          }
        } catch {
          if (currentSessionRef.current === sid && isProcessingRef.current) {
            connectStream(sid, true);
          }
        }
      }, delay);
    };
  };

  const updateLastAssistant = (updater: (msg: Message) => Message) => {
    setMessages(prev => {
      const copy = [...prev];
      for (let i = copy.length - 1; i >= 0; i--) {
        if (copy[i].role === 'assistant') {
          copy[i] = updater(copy[i]);
          break;
        }
      }
      return copy;
    });
  };

  // Periodic session-state sync fallback:
  // If SSE misses terminal events, recover by reading persisted session state.
  useEffect(() => {
    if (!isProcessing) return;

    const timer = window.setInterval(async () => {
      const sid = currentSessionRef.current;
      if (!sid || !isProcessingRef.current || sessionSyncInFlightRef.current) {
        return;
      }

      sessionSyncInFlightRef.current = true;
      try {
        const detail = await chatApi.getSession(sid);
        if (currentSessionRef.current !== sid) return;

        if (!detail.is_processing) {
          const parsed = parseMessages(detail.messages_json);
          if (parsed.length > 0) {
            setMessages(parsed);
          } else {
            updateLastAssistant(msg => ({ ...msg, isStreaming: false }));
          }

          eventSourceRef.current?.close();
          eventSourceRef.current = null;
          setIsProcessing(false);
          isProcessingRef.current = false;
          const completedLabel = t('chat.completed', 'Completed');
          setLiveStatus(completedLabel);
          emitRuntimeEvent('done', completedLabel, { source: 'session_poll' });
        }
      } catch {
        // Ignore transient polling failures; SSE/retry flow remains primary.
      } finally {
        sessionSyncInFlightRef.current = false;
      }
    }, 5000);

    return () => window.clearInterval(timer);
  }, [emitRuntimeEvent, isProcessing, t]);

  const handleStop = useCallback(async () => {
    const sid = currentSessionRef.current;
    if (!sid) return;
    try {
      await chatApi.cancelChat(sid);
      const cancelledLabel = t('chat.cancelled', 'Cancelled');
      setLiveStatus(cancelledLabel);
      setIsProcessing(false);
      isProcessingRef.current = false;
      emitRuntimeEvent('done', cancelledLabel);
    } catch (e) {
      console.error('Failed to cancel:', e);
    }
  }, [emitRuntimeEvent, t]);

  if (loading) {
    return (
      <div className="flex-1 flex items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="flex flex-col flex-1 min-w-0 min-h-0">
      {/* Header with agent info */}
      <div className="shadow-[0_1px_2px_0_rgba(0,0,0,0.05)]">
        <div className="px-4 py-2.5 flex items-center gap-3">
          <div className="w-8 h-8 rounded-full bg-muted-foreground/15 flex items-center justify-center shrink-0">
            <Bot className="w-4 h-4 text-muted-foreground" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="font-medium text-sm">{agentName}</span>
              <span className={`h-2 w-2 rounded-full shrink-0 ${
                agent?.status === 'running' ? 'bg-green-500' :
                agent?.status === 'error' ? 'bg-red-500' :
                agent?.status === 'paused' ? 'bg-amber-500' : 'bg-slate-400'
              }`} />
              {agent?.model && (
                <span className="text-[11px] bg-muted text-muted-foreground rounded px-1.5 py-0.5">
                  {agent.model}
                </span>
              )}
            </div>
            {agent?.description && (
              <p className="text-xs text-muted-foreground truncate">{agent.description}</p>
            )}
          </div>
          {agent && (agent.assigned_skills?.length > 0 || agent.enabled_extensions?.length > 0) && (
            <button
              onClick={() => setShowCapabilities(!showCapabilities)}
              className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 shrink-0"
            >
              {showCapabilities ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
              {t('chat.capabilities', 'Capabilities')}
            </button>
          )}
        </div>
        {/* Expandable capabilities panel */}
        {showCapabilities && agent && (
          <div className="px-4 pb-3 flex flex-wrap gap-1.5 pt-2 bg-muted/30">
            {agent.assigned_skills?.filter(s => s.enabled).map(skill => (
              <span key={skill.skill_id} className="inline-flex items-center gap-1 text-[11px] bg-background border rounded-full px-2 py-0.5">
                <Zap className="h-3 w-3 text-amber-500" />
                {skill.name}
              </span>
            ))}
            {agent.enabled_extensions?.filter(e => e.enabled).map(ext => (
              <span key={ext.extension} className="inline-flex items-center gap-1 text-[11px] bg-background border rounded-full px-2 py-0.5">
                <Puzzle className="h-3 w-3 text-blue-500" />
                {ext.extension}
              </span>
            ))}
          </div>
        )}
      </div>

      {/* Live execution status */}
      {isProcessing && (
        <div className="mx-4 mt-3 mb-1 rounded-md border bg-muted/40 px-3 py-2 text-xs text-muted-foreground flex items-center justify-between gap-3">
          <span className="truncate">{liveStatus || t('chat.processing', 'Processing...')}</span>
          <span className="shrink-0">{t('chat.elapsed', '{{n}}s', { n: elapsedSeconds })}</span>
        </div>
      )}

      {/* Messages */}
      <div ref={scrollContainerRef} className="flex-1 overflow-y-auto p-4">
        {messages.length === 0 && !isProcessing && (
          <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
            {t('chat.startConversation', 'Send a message to start the conversation')}
          </div>
        )}
        {messages.map((msg) => (
          <ChatMessageBubble key={msg.id} {...msg} agentName={agentName} userName={user?.display_name} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      {/* Attached documents chips */}
      {(attachedDocs.length > 0 || pendingDocIds.length > 0) && (
        <div className="flex items-center gap-1 px-4 pt-2 flex-wrap">
          {attachedDocs.map(doc => (
            <span key={doc.id} className="inline-flex items-center gap-1 text-xs bg-muted px-2 py-1 rounded-full">
              {doc.display_name || doc.name}
              <button onClick={() => {
                setAttachedDocs(prev => prev.filter(d => d.id !== doc.id));
                setPendingDocIds(prev => prev.filter(id => id !== doc.id));
              }}>
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* Input with attach button */}
      <div className="flex items-end gap-1">
        {teamId && (
          <button
            onClick={() => setShowDocPicker(true)}
            className="p-2 mb-4 ml-2 rounded-md hover:bg-muted text-muted-foreground"
            title={t('documents.attachDocuments')}
          >
            <Paperclip className="h-4 w-4" />
          </button>
        )}
        <div className="flex-1">
          <ChatInput
            onSend={handleSend}
            onStop={handleStop}
            isProcessing={isProcessing}
          />
        </div>
      </div>

      {/* Document Picker Dialog */}
      {teamId && (
        <DocumentPicker
          teamId={teamId}
          open={showDocPicker}
          onClose={() => setShowDocPicker(false)}
          onSelect={(docs) => {
            setAttachedDocs(prev => {
              const existingIds = new Set(prev.map(d => d.id));
              return [...prev, ...docs.filter(d => !existingIds.has(d.id))];
            });
            setPendingDocIds(prev => {
              const existingIds = new Set(prev);
              return [...prev, ...docs.map(d => d.id).filter(id => !existingIds.has(id))];
            });
          }}
          selectedIds={pendingDocIds}
        />
      )}
    </div>
  );
}
