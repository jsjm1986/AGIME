/**
 * ProgressiveMessageList Component
 *
 * A performance-optimized message list that renders messages progressively
 * starting from the LATEST messages (tail-first loading). This provides
 * immediate visibility of recent conversations while lazily loading history.
 *
 * Key Features:
 * - Tail-first rendering: Shows newest messages immediately
 * - Lazy history loading: Loads older messages when scrolling to top
 * - Scroll position preservation: Maintains reading position during history load
 * - Search compatibility: Ctrl/Cmd+F loads all messages instantly
 * - Configurable initial count and batch size
 */
/* eslint-disable no-undef */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2 } from 'lucide-react';
import { Message } from '../api';
import AgimeMessage from './AgimeMessage';
import UserMessage from './UserMessage';
import { SystemNotificationInline } from './context_management/SystemNotificationInline';
import { NotificationEvent } from '../types/message';
import { ChatType } from '../types/chat';
import { identifyConsecutiveToolCalls, isInChain } from '../utils/toolCallChaining';

interface ProgressiveMessageListProps {
  messages: Message[];
  chat?: Pick<ChatType, 'sessionId' | 'messageHistoryIndex'>;
  toolCallNotifications?: Map<string, NotificationEvent[]>;
  append?: (value: string) => void;
  isUserMessage: (message: Message) => boolean;
  initialVisibleCount?: number; // Number of messages to show initially (from the end)
  batchSize?: number; // Number of messages to load when scrolling up
  showLoadingThreshold?: number; // Only enable lazy loading if more than X messages
  renderMessage?: (message: Message, index: number) => React.ReactNode | null;
  isStreamingMessage?: boolean;
  onMessageUpdate?: (messageId: string, newContent: string) => void;
  onRenderingComplete?: () => void;
  submitElicitationResponse?: (
    elicitationId: string,
    userData: Record<string, unknown>
  ) => Promise<void>;
}

export default function ProgressiveMessageList({
  messages,
  chat,
  toolCallNotifications = new Map(),
  append = () => {},
  isUserMessage,
  initialVisibleCount = 50, // Show latest 50 messages initially
  batchSize = 30, // Load 30 more when scrolling up
  showLoadingThreshold = 80, // Only lazy load if more than 80 messages
  renderMessage,
  isStreamingMessage = false,
  onMessageUpdate,
  onRenderingComplete,
  submitElicitationResponse,
}: ProgressiveMessageListProps) {
  const { t } = useTranslation('chat');

  // Calculate visible range (from the end)
  const [visibleStartIndex, setVisibleStartIndex] = useState(() => {
    if (messages.length <= showLoadingThreshold) {
      return 0; // Show all messages if below threshold
    }
    return Math.max(0, messages.length - initialVisibleCount);
  });

  const [isLoadingHistory, setIsLoadingHistory] = useState(false);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const mountedRef = useRef(true);
  const isLoadingRef = useRef(false); // Prevent concurrent loads

  // Check if there's more history to load
  const hasMoreHistory = visibleStartIndex > 0;

  // Helper functions
  const hasOnlyToolResponses = (message: Message) =>
    message.content.every((c) => c.type === 'toolResponse');

  const hasInlineSystemNotification = (message: Message): boolean => {
    return message.content.some(
      (content) =>
        content.type === 'systemNotification' && content.notificationType === 'inlineMessage'
    );
  };

  // Reset visible range when messages change significantly (e.g., new session)
  useEffect(() => {
    if (messages.length <= showLoadingThreshold) {
      setVisibleStartIndex(0);
    } else {
      // For large message lists, start from the end
      setVisibleStartIndex(Math.max(0, messages.length - initialVisibleCount));
    }
  }, [messages.length, showLoadingThreshold, initialVisibleCount]);

  // Call rendering complete callback
  useEffect(() => {
    if (onRenderingComplete) {
      // Small delay to ensure DOM is updated
      const timer = setTimeout(() => onRenderingComplete(), 50);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [onRenderingComplete, visibleStartIndex]);

  // Load more history when sentinel becomes visible
  const loadMoreHistory = useCallback(() => {
    if (isLoadingRef.current || visibleStartIndex <= 0) return;

    isLoadingRef.current = true;
    setIsLoadingHistory(true);

    // Get scroll container by finding the closest scroll viewport (works with Radix ScrollArea)
    const scrollContainer = sentinelRef.current?.closest('[data-radix-scroll-area-viewport]') as HTMLElement | null;
    const scrollHeightBefore = scrollContainer?.scrollHeight || 0;
    const scrollTopBefore = scrollContainer?.scrollTop || 0;

    // Calculate new start index
    const newStartIndex = Math.max(0, visibleStartIndex - batchSize);

    // Use requestAnimationFrame for smoother updates
    requestAnimationFrame(() => {
      setVisibleStartIndex(newStartIndex);

      // Preserve scroll position after DOM update
      requestAnimationFrame(() => {
        if (scrollContainer) {
          const scrollHeightAfter = scrollContainer.scrollHeight;
          const heightDiff = scrollHeightAfter - scrollHeightBefore;
          scrollContainer.scrollTop = scrollTopBefore + heightDiff;
        }

        setIsLoadingHistory(false);
        isLoadingRef.current = false;
      });
    });
  }, [visibleStartIndex, batchSize]);

  // IntersectionObserver to detect when user scrolls to top
  useEffect(() => {
    if (!hasMoreHistory || !sentinelRef.current) return;

    const observer = new IntersectionObserver(
      (entries) => {
        const [entry] = entries;
        if (entry.isIntersecting && !isLoadingRef.current) {
          loadMoreHistory();
        }
      },
      {
        rootMargin: '200px 0px 0px 0px', // Trigger 200px before reaching top
        threshold: 0,
      }
    );

    observer.observe(sentinelRef.current);

    return () => observer.disconnect();
  }, [hasMoreHistory, loadMoreHistory]);

  // Cleanup on unmount
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Force load all messages when search is triggered
  useEffect(() => {
    if (visibleStartIndex === 0) return; // Already showing all

    const handleKeyDown = (e: KeyboardEvent) => {
      const isMac = window.electron.platform === 'darwin';
      const isSearchShortcut = (isMac ? e.metaKey : e.ctrlKey) && e.key === 'f';

      if (isSearchShortcut) {
        // Immediately show all messages for search
        setVisibleStartIndex(0);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [visibleStartIndex]);

  // Detect tool call chains (for the full message list)
  const toolCallChains = useMemo(() => identifyConsecutiveToolCalls(messages), [messages]);

  // Get visible messages
  const visibleMessages = useMemo(
    () => messages.slice(visibleStartIndex),
    [messages, visibleStartIndex]
  );

  // Render messages
  const renderMessages = useCallback(() => {
    return visibleMessages
      .map((message, localIndex) => {
        // Calculate the actual index in the full messages array
        const actualIndex = visibleStartIndex + localIndex;

        if (!message.metadata.userVisible) {
          return null;
        }

        if (renderMessage) {
          return renderMessage(message, actualIndex);
        }

        // Default rendering logic (for BaseChat)
        if (!chat) {
          console.warn(
            'ProgressiveMessageList: chat prop is required when not using custom renderMessage'
          );
          return null;
        }

        // System notifications are never user messages, handle them first
        if (hasInlineSystemNotification(message)) {
          return (
            <div
              key={message.id ?? `msg-${actualIndex}-${message.created}`}
              className={`relative ${localIndex === 0 ? 'mt-0' : 'mt-4'} assistant`}
              data-testid="message-container"
            >
              <SystemNotificationInline message={message} />
            </div>
          );
        }

        const isUser = isUserMessage(message);
        const messageIsInChain = isInChain(actualIndex, toolCallChains);

        return (
          <div
            key={message.id ?? `msg-${actualIndex}-${message.created}`}
            className={`relative ${localIndex === 0 ? 'mt-0' : 'mt-4'} ${isUser ? 'user' : 'assistant'} ${messageIsInChain ? 'in-chain' : ''}`}
            data-testid="message-container"
          >
            {isUser ? (
              !hasOnlyToolResponses(message) && (
                <UserMessage message={message} onMessageUpdate={onMessageUpdate} />
              )
            ) : (
              <AgimeMessage
                sessionId={chat.sessionId}
                messageHistoryIndex={chat.messageHistoryIndex}
                message={message}
                messages={messages}
                append={append}
                toolCallNotifications={toolCallNotifications}
                isStreaming={
                  isStreamingMessage &&
                  !isUser &&
                  localIndex === visibleMessages.length - 1 &&
                  message.role === 'assistant'
                }
                submitElicitationResponse={submitElicitationResponse}
              />
            )}
          </div>
        );
      })
      .filter(Boolean);
  }, [
    visibleMessages,
    visibleStartIndex,
    renderMessage,
    isUserMessage,
    chat,
    append,
    toolCallNotifications,
    isStreamingMessage,
    onMessageUpdate,
    toolCallChains,
    messages,
    submitElicitationResponse,
  ]);

  return (
    <>
      {/* Top sentinel for lazy loading history */}
      {hasMoreHistory && (
        <div ref={sentinelRef} className="flex items-center justify-center py-4">
          {isLoadingHistory ? (
            <div className="flex items-center gap-2 text-sm text-textSubtle">
              <Loader2 className="h-4 w-4 animate-spin" />
              <span>{t('progressiveLoading.loadingHistory')}</span>
            </div>
          ) : (
            <div className="text-xs text-textSubtle">
              {t('progressiveLoading.olderMessages', { count: visibleStartIndex })}
            </div>
          )}
        </div>
      )}

      {renderMessages()}
    </>
  );
}
