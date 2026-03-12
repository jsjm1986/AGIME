import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { Loader2, Paperclip, Upload, X, Bot, ChevronDown, ChevronRight, Zap, Puzzle, Wrench } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useAuth } from '../../contexts/AuthContext';
import { chatApi, type CreateSessionOptions } from '../../api/chat';
import { documentApi, type DocumentSummary } from '../../api/documents';
import { ChatMessageBubble } from './ChatMessageBubble';
import { ChatInput, type ChatInputComposeRequest, type ChatInputQuickActionGroup } from './ChatInput';
import { DocumentPicker } from '../documents/DocumentPicker';
import type { TeamAgent } from '../../api/agent';
import type { Message } from './ChatMessageBubble';

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50MB
const CHAT_DEBUG_VIEW_STORAGE_KEY = 'chat:show_tool_debug_messages:v1';

const AGENT_STATUS_DOT: Record<string, string> = {
  running: 'bg-status-success-text',
  error: 'bg-status-error-text',
  paused: 'bg-status-warning-text',
};

const FILE_ACCEPT = [
  '.pdf', '.doc', '.docx', '.xls', '.xlsx', '.ppt', '.pptx',
  '.txt', '.md', '.csv', '.json', '.xml', '.html', '.htm', '.rtf',
  '.odt', '.ods', '.odp',
  '.png', '.jpg', '.jpeg', '.gif', '.webp', '.svg',
].join(',');

export interface ChatRuntimeEvent {
  kind: 'status' | 'turn' | 'toolcall' | 'toolresult' | 'compaction' | 'workspace_changed' | 'done' | 'connection' | 'goal' | 'text';
  text: string;
  ts: number;
  detail?: Record<string, unknown>;
}

interface ChatConversationProps {
  sessionId: string | null;
  agentId: string;
  agentName: string;
  agent?: TeamAgent | null;
  headerVariant?: 'default' | 'compact';
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
  /** Optional compose request from parent (prefill or auto-send) */
  composeRequest?: ChatInputComposeRequest | null;
  inputQuickActionGroups?: ChatInputQuickActionGroup[];
}

function extractTaggedThinking(source: string): { content: string; thinking: string } {
  if (!source) {
    return { content: '', thinking: '' };
  }

  const lower = source.toLowerCase();
  const contentParts: string[] = [];
  const thinkingParts: string[] = [];
  let cursor = 0;

  while (cursor < source.length) {
    const thinkIndex = lower.indexOf('<think>', cursor);
    const thinkingIndex = lower.indexOf('<thinking>', cursor);
    const candidates = [thinkIndex, thinkingIndex].filter((index) => index >= 0);

    if (candidates.length === 0) {
      contentParts.push(source.slice(cursor));
      break;
    }

    const openIndex = Math.min(...candidates);
    contentParts.push(source.slice(cursor, openIndex));

    const usesLongTag = thinkingIndex >= 0 && thinkingIndex === openIndex;
    const openTag = usesLongTag ? '<thinking>' : '<think>';
    const closeTag = usesLongTag ? '</thinking>' : '</think>';
    const innerStart = openIndex + openTag.length;
    const closeIndex = lower.indexOf(closeTag, innerStart);

    if (closeIndex === -1) {
      thinkingParts.push(source.slice(innerStart));
      break;
    }

    thinkingParts.push(source.slice(innerStart, closeIndex));
    cursor = closeIndex + closeTag.length;
  }

  return {
    content: contentParts.join(''),
    thinking: thinkingParts.join(''),
  };
}

function combineThinkingSegments(...segments: Array<string | null | undefined>): string | undefined {
  const normalized = segments
    .map((segment) => (segment || '').trim())
    .filter((segment) => segment.length > 0);
  if (normalized.length === 0) {
    return undefined;
  }
  return normalized.join('\n');
}

function deriveAssistantPresentation(rawContent?: string, rawThinking?: string) {
  const extracted = extractTaggedThinking(rawContent || '');
  return {
    content: extracted.content,
    thinking: combineThinkingSegments(rawThinking, extracted.thinking),
  };
}

export function ChatConversation({
  sessionId,
  agentId,
  agentName,
  agent,
  headerVariant = 'default',
  teamId,
  initialAttachedDocIds,
  createSession,
  createSessionOptions,
  onSessionCreated,
  onToolResult,
  onProcessingChange,
  onRuntimeEvent,
  onError,
  composeRequest,
  inputQuickActionGroups,
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
  const [uploading, setUploading] = useState(false);
  const uploadingRef = useRef(false);
  const [showCapabilities, setShowCapabilities] = useState(false);
  const [showToolDebugMessages, setShowToolDebugMessages] = useState<boolean>(() => {
    try {
      return window.localStorage.getItem(CHAT_DEBUG_VIEW_STORAGE_KEY) === '1';
    } catch {
      return false;
    }
  });
  const fileInputRef = useRef<HTMLInputElement>(null);
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

  useEffect(() => {
    if (sessionId) {
      return;
    }
    setPendingDocIds(initialAttachedDocIds || []);
  }, [initialAttachedDocIds, sessionId]);

  useEffect(() => {
    if (!teamId || pendingDocIds.length === 0) {
      if (!sessionId) {
        setAttachedDocs([]);
      }
      return;
    }
    const missingIds = pendingDocIds.filter((id) => !attachedDocs.some((doc) => doc.id === id));
    if (missingIds.length === 0) {
      return;
    }
    let cancelled = false;
    documentApi.getDocumentsByIds(teamId, missingIds)
      .then((docs) => {
        if (cancelled) {
          return;
        }
        setAttachedDocs((prev) => {
          const existingIds = new Set(prev.map((doc) => doc.id));
          return [...prev, ...docs.filter((doc) => !existingIds.has(doc.id))];
        });
      })
      .catch((error) => {
        if (!cancelled) {
          console.error('Failed to resolve attached documents:', error);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [attachedDocs, pendingDocIds, sessionId, teamId]);

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

  useEffect(() => {
    try {
      window.localStorage.setItem(
        CHAT_DEBUG_VIEW_STORAGE_KEY,
        showToolDebugMessages ? '1' : '0'
      );
    } catch {
      // Ignore localStorage failures (private mode, etc.)
    }
  }, [showToolDebugMessages]);

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
            rawContent: '',
            rawThinking: '',
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

        let rawText = '';
        let rawThinking = '';
        const toolCalls: Array<{ name: string; id: string }> = [];
        if (typeof m.content === 'string') {
          rawText = m.content;
        } else if (Array.isArray(m.content)) {
          for (const c of m.content) {
            const cType = String(c?.type || '').toLowerCase();
            if (cType === 'text' || (!cType && c?.text)) {
              rawText += c?.text || '';
              continue;
            }
            if (cType === 'thinking' && c?.thinking) {
              rawThinking += c.thinking;
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
              rawText += c.msg;
            }
          }
        } else if (m.content && typeof m.content === 'object') {
          const c = m.content as Record<string, unknown>;
          if (typeof c.text === 'string') {
            rawText += c.text;
          } else if (typeof c.msg === 'string') {
            rawText += c.msg;
          }
        }

        const visibleAssistant =
          role === 'assistant'
            ? deriveAssistantPresentation(rawText, rawThinking)
            : { content: rawText, thinking: undefined as string | undefined };

        const hasContent =
          visibleAssistant.content.trim().length > 0 ||
          (visibleAssistant.thinking || '').trim().length > 0 ||
          toolCalls.length > 0;
        if (!hasContent) continue;
        const createdRaw = Number(m?.created ?? m?.timestamp ?? 0);
        const createdMs = Number.isFinite(createdRaw)
          ? (createdRaw > 10_000_000_000 ? createdRaw : createdRaw * 1000)
          : 0;
        parsed.push({
          id: `hist-${i}`,
          role,
          content: visibleAssistant.content,
          thinking: visibleAssistant.thinking,
          rawContent: role === 'assistant' ? rawText : undefined,
          rawThinking: role === 'assistant' ? rawThinking : undefined,
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

  const handleFileUpload = useCallback(async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files?.length || !teamId || uploadingRef.current) return;
    uploadingRef.current = true;
    setUploading(true);
    try {
      for (const file of Array.from(files)) {
        if (file.size > MAX_FILE_SIZE) {
          onError?.(`${file.name}: ${t('documents.fileTooLarge', 'File exceeds 50MB limit')}`);
          continue;
        }
        try {
          const doc = await documentApi.uploadDocument(teamId, file);
          setAttachedDocs(prev => prev.some(d => d.id === doc.id) ? prev : [...prev, doc]);
          setPendingDocIds(prev => prev.includes(doc.id) ? prev : [...prev, doc.id]);
        } catch (err: unknown) {
          const msg = err instanceof Error ? err.message : String(err);
          onError?.(msg || `${file.name} ${t('documents.upload')} failed`);
        }
      }
    } finally {
      uploadingRef.current = false;
      setUploading(false);
      if (fileInputRef.current) fileInputRef.current.value = '';
    }
  }, [teamId, onError, t]);

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
      {
        id: assistantMsgId,
        role: 'assistant' as const,
        content: '',
        rawContent: '',
        rawThinking: '',
        isStreaming: true,
        timestamp: new Date(),
      },
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
    try {
      const parsed = JSON.parse(raw || '{}');
      if (parsed?.type === 'tool_task_progress') {
        const tool = parsed.tool_name || parsed.task_id || 'tool';
        const status = parsed.status || 'working';
        const msg = parsed.status_message ? ` - ${parsed.status_message}` : '';
        return `${tool}: ${status}${msg}`;
      }
    } catch {
      // Non-JSON status; use legacy matching below.
    }
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
      if (typeof data.content === 'string' && data.content.length > 0) {
        emitRuntimeEvent('text', data.content, { source: 'assistant_stream' });
      }
      updateLastAssistant(msg => {
        const nextRawContent = (msg.rawContent || '') + (typeof data.content === 'string' ? data.content : '');
        const derived = deriveAssistantPresentation(nextRawContent, msg.rawThinking || '');
        return {
          ...msg,
          rawContent: nextRawContent,
          content: derived.content,
          thinking: derived.thinking,
        };
      });
    });

    es.addEventListener('thinking', (e) => {
      if (currentSessionRef.current !== sid) return;
      captureEventId(e);
      const data = safeParse(e.data);
      if (!data) return;
      updateLastAssistant(msg => {
        const nextRawThinking = (msg.rawThinking || '') + (typeof data.content === 'string' ? data.content : '');
        const derived = deriveAssistantPresentation(msg.rawContent || '', nextRawThinking);
        return {
          ...msg,
          rawThinking: nextRawThinking,
          content: derived.content,
          thinking: derived.thinking,
        };
      });
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

  const displayMessages = useMemo(() => {
    if (showToolDebugMessages) return messages;

    const out: Message[] = [];
    for (const msg of messages) {
      const isToolOnlyAssistant =
        msg.role === 'assistant' &&
        (msg.content || '').trim().length === 0 &&
        !(msg.thinking && msg.thinking.trim().length > 0) &&
        (msg.toolCalls?.length || 0) > 0;

      if (!isToolOnlyAssistant) {
        out.push(msg);
        continue;
      }

      // In compact mode, merge standalone tool-only bubbles into the nearest
      // previous assistant bubble to avoid noisy "one tool = one bubble".
      let merged = false;
      for (let i = out.length - 1; i >= 0; i -= 1) {
        if (out[i].role !== 'assistant') continue;
        out[i] = {
          ...out[i],
          toolCalls: [...(out[i].toolCalls || []), ...(msg.toolCalls || [])],
          turn: msg.turn || out[i].turn,
          compaction: msg.compaction || out[i].compaction,
        };
        merged = true;
        break;
      }

      if (!merged) {
        // No suitable assistant bubble yet; keep one compact synthetic bubble.
        out.push({
          ...msg,
          id: `${msg.id}-compact`,
          content: t('chat.toolRunSummary', '工具执行摘要'),
        });
      }
    }
    return out;
  }, [messages, showToolDebugMessages, t]);

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

  const normalizedAgentName = agentName.trim().toLowerCase();
  const normalizedModelName = (agent?.model || '').trim().toLowerCase();
  const showModelBadge = !!agent?.model && normalizedModelName !== normalizedAgentName;
  const hasSecondaryIdentity = !!agent?.description || showModelBadge;
  const compactHeader = headerVariant === 'compact';

  if (loading) {
    return (
      <div className="flex h-full min-h-0 min-w-0 flex-1 items-center justify-center">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col">
      {/* Header with agent info */}
      <div className="border-b bg-background/95 backdrop-blur-sm">
        <div className={`px-4 ${compactHeader ? 'py-1.5' : 'py-2.5'} flex ${compactHeader ? 'items-center gap-2' : 'items-start gap-3'} min-w-0`}>
          <div className={`${compactHeader ? 'h-7 w-7' : 'w-8 h-8'} rounded-full bg-muted-foreground/15 flex items-center justify-center shrink-0`}>
            <Bot className={`${compactHeader ? 'h-3.5 w-3.5' : 'w-4 h-4'} text-muted-foreground`} />
          </div>
          <div className={`flex-1 min-w-0 ${compactHeader ? '' : 'space-y-0.5'}`}>
            <div className={`flex items-center min-w-0 ${compactHeader ? 'gap-1.5' : 'gap-2'}`}>
              <span className={`font-medium truncate ${compactHeader ? 'text-[12px] leading-4' : 'text-[13px] leading-5'}`}>{agentName}</span>
              <span className={`h-2 w-2 rounded-full shrink-0 ${
                AGENT_STATUS_DOT[agent?.status || ''] || 'bg-status-neutral-text'
              }`} />
              {showModelBadge && (
                <span className={`hidden sm:inline-flex items-center bg-muted text-muted-foreground rounded px-1.5 py-0.5 shrink-0 ${compactHeader ? 'text-[10px]' : 'text-caption'}`}>
                  {agent.model}
                </span>
              )}
            </div>
            {!compactHeader && hasSecondaryIdentity && (
              <div className="flex items-center gap-1.5 min-h-[18px] min-w-0">
                {showModelBadge && (
                  <span className="sm:hidden inline-flex items-center text-caption bg-muted text-muted-foreground rounded px-1.5 py-0.5 shrink-0">
                    {agent.model}
                  </span>
                )}
                {agent?.description && (
                  <p className="text-caption text-muted-foreground truncate">{agent.description}</p>
                )}
              </div>
            )}
          </div>
          <div className={`ml-auto flex items-center shrink-0 ${compactHeader ? 'gap-1' : 'gap-1.5'}`}>
            {!compactHeader && agent && (agent.assigned_skills?.length > 0 || agent.enabled_extensions?.length > 0) && (
              <button
                onClick={() => setShowCapabilities(!showCapabilities)}
                className="h-7 inline-flex items-center gap-1 rounded-md border border-border/60 px-2 text-caption text-muted-foreground hover:text-foreground hover:bg-muted/40 transition-colors"
              >
                {showCapabilities ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
                <span className="hidden md:inline">{t('chat.capabilities', 'Capabilities')}</span>
                <span className="md:hidden">{t('chat.capabilitiesShort', '能力')}</span>
              </button>
            )}
            <button
              onClick={() => setShowToolDebugMessages(v => !v)}
              className={`${compactHeader ? 'h-6 w-6 justify-center rounded-md p-0' : 'h-7 gap-1 rounded-md px-2 text-caption'} inline-flex items-center border transition-colors ${
                showToolDebugMessages
                  ? 'text-foreground border-border bg-muted/60'
                  : 'text-muted-foreground border-border/50 hover:text-foreground hover:bg-muted/40'
              }`}
              title={showToolDebugMessages
                ? t('chat.switchCompact', '切换为简洁模式')
                : t('chat.switchDebug', '切换为调试模式')}
            >
              <Wrench className="h-3.5 w-3.5" />
              {!compactHeader && (
              <span className="hidden sm:inline">
                {showToolDebugMessages
                  ? t('chat.debugModeOn', '调试模式')
                  : t('chat.compactModeOn', '简洁模式')}
              </span>
              )}
            </button>
          </div>
        </div>
        {/* Expandable capabilities panel */}
        {!compactHeader && showCapabilities && agent && (
          <div className="px-4 pb-3 flex flex-wrap gap-1.5 pt-2 bg-muted/30">
            {agent.assigned_skills?.filter(s => s.enabled).map(skill => (
              <span key={skill.skill_id} className="inline-flex items-center gap-1 text-caption bg-background border rounded-full px-2 py-0.5">
                <Zap className="h-3 w-3 text-amber-500" />
                {skill.name}
              </span>
            ))}
            {agent.enabled_extensions?.filter(e => e.enabled).map(ext => (
              <span key={ext.extension} className="inline-flex items-center gap-1 text-caption bg-background border rounded-full px-2 py-0.5">
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
      <div ref={scrollContainerRef} className="flex-1 overflow-y-auto overflow-x-hidden p-4">
        {messages.length === 0 && !isProcessing && (
          <div className="flex items-center justify-center h-full text-muted-foreground text-[13px]">
            {t('chat.startConversation', 'Send a message to start the conversation')}
          </div>
        )}
        {displayMessages.map((msg) => (
          <ChatMessageBubble
            key={msg.id}
            {...msg}
            agentName={agentName}
            userName={user?.display_name}
            autoExpandTools={showToolDebugMessages}
          />
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
          <div className="flex items-center mb-4 ml-2 gap-0.5">
            <button
              onClick={() => setShowDocPicker(true)}
              className="p-2 rounded-md hover:bg-muted text-muted-foreground"
              title={t('documents.attachDocuments')}
              aria-label={t('documents.attachDocuments')}
            >
              <Paperclip className="h-4 w-4" />
            </button>
            <button
              onClick={() => fileInputRef.current?.click()}
              disabled={uploading}
              className="p-2 rounded-md hover:bg-muted text-muted-foreground disabled:opacity-50"
              title={t('documents.upload')}
              aria-label={t('documents.upload')}
            >
              {uploading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
            </button>
            <input
              ref={fileInputRef}
              type="file"
              accept={FILE_ACCEPT}
              multiple
              className="hidden"
              onChange={handleFileUpload}
            />
          </div>
        )}
        <div className="flex-1">
          <ChatInput
            onSend={handleSend}
            onStop={handleStop}
            isProcessing={isProcessing}
            composeRequest={composeRequest}
            quickActionGroups={inputQuickActionGroups}
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
