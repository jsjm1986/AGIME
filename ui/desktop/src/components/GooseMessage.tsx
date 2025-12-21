import { useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import ImagePreview from './ImagePreview';
import { extractImagePaths, removeImagePathsFromText } from '../utils/imageUtils';
import { formatMessageTimestamp } from '../utils/timeUtils';
import MarkdownContent from './MarkdownContent';
import ToolCallWithResponse from './ToolCallWithResponse';
import ThinkingBlock from './ThinkingBlock';
import {
  getTextContent,
  getToolRequests,
  getToolResponses,
  getToolConfirmationContent,
  getElicitationContent,
  NotificationEvent,
} from '../types/message';
import { Message, confirmToolAction, ThinkingContent } from '../api';
import ToolCallConfirmation from './ToolCallConfirmation';
import ElicitationRequest from './ElicitationRequest';
import MessageCopyLink from './MessageCopyLink';
import { cn } from '../utils';
import { identifyConsecutiveToolCalls, shouldHideTimestamp } from '../utils/toolCallChaining';
import { useThinkingVisibility } from '../contexts/ThinkingVisibilityContext';

// Extract thinking content from message (Claude Extended Thinking)
function getThinkingContent(message: Message): ThinkingContent | undefined {
  for (const content of message.content) {
    if (content.type === 'thinking') {
      return content as ThinkingContent & { type: 'thinking' };
    }
  }
  return undefined;
}

interface GooseMessageProps {
  // messages up to this index are presumed to be "history" from a resumed session, this is used to track older tool confirmation requests
  // anything before this index should not render any buttons, but anything after should
  sessionId: string;
  messageHistoryIndex: number;
  message: Message;
  messages: Message[];
  metadata?: string[];
  toolCallNotifications: Map<string, NotificationEvent[]>;
  append: (value: string) => void;
  isStreaming?: boolean; // Whether this message is currently being streamed
  submitElicitationResponse?: (
    elicitationId: string,
    userData: Record<string, unknown>
  ) => Promise<void>;
}

export default function GooseMessage({
  sessionId,
  messageHistoryIndex,
  message,
  messages,
  toolCallNotifications,
  append,
  isStreaming = false,
  submitElicitationResponse,
}: GooseMessageProps) {
  const { t } = useTranslation('chat');
  const { showThinking } = useThinkingVisibility();
  const contentRef = useRef<HTMLDivElement | null>(null);
  const handledToolConfirmations = useRef<Set<string>>(new Set());

  let textContent = getTextContent(message);

  const splitChainOfThought = (text: string): { visibleText: string; cotText: string | null } => {
    const regex = /<think>([\s\S]*?)<\/think>/i;
    const match = text.match(regex);
    if (!match) {
      return { visibleText: text, cotText: null };
    }

    const cotRaw = match[1].trim();
    const visibleText = text.replace(regex, '').trim();

    return {
      visibleText,
      cotText: cotRaw || null,
    };
  };

  const { visibleText, cotText } = splitChainOfThought(textContent);

  // Get Claude Extended Thinking content (type: 'thinking')
  const thinkingContent = getThinkingContent(message);
  // Combine both thinking sources
  const allThinkingText = thinkingContent?.thinking || cotText;

  const imagePaths = extractImagePaths(visibleText);
  const displayText =
    imagePaths.length > 0 ? removeImagePathsFromText(visibleText, imagePaths) : visibleText;

  const timestamp = useMemo(() => formatMessageTimestamp(message.created), [message.created]);
  const toolRequests = getToolRequests(message);
  const messageIndex = messages.findIndex((msg) => msg.id === message.id);
  const toolConfirmationContent = getToolConfirmationContent(message);
  const elicitationContent = getElicitationContent(message);
  const toolCallChains = useMemo(() => identifyConsecutiveToolCalls(messages), [messages]);
  const hideTimestamp = useMemo(
    () => shouldHideTimestamp(messageIndex, toolCallChains),
    [messageIndex, toolCallChains]
  );
  const hasToolConfirmation = toolConfirmationContent !== undefined;
  const hasElicitation = elicitationContent !== undefined;

  const toolResponsesMap = useMemo(() => {
    const responseMap = new Map();

    if (messageIndex !== undefined && messageIndex >= 0) {
      for (let i = messageIndex + 1; i < messages.length; i++) {
        const responses = getToolResponses(messages[i]);

        for (const response of responses) {
          const matchingRequest = toolRequests.find((req) => req.id === response.id);
          if (matchingRequest) {
            responseMap.set(response.id, response);
          }
        }
      }
    }

    return responseMap;
  }, [messages, messageIndex, toolRequests]);

  useEffect(() => {
    if (
      messageIndex === messageHistoryIndex - 1 &&
      hasToolConfirmation &&
      toolConfirmationContent &&
      !handledToolConfirmations.current.has(toolConfirmationContent.data.id)
    ) {
      const hasExistingResponse = messages.some((msg) =>
        getToolResponses(msg).some((response) => response.id === toolConfirmationContent.data.id)
      );

      if (!hasExistingResponse) {
        handledToolConfirmations.current.add(toolConfirmationContent.data.id);

        void (async () => {
          try {
            await confirmToolAction({
              body: {
                sessionId,
                action: 'deny',
                id: toolConfirmationContent.data.id,
              },
              throwOnError: true,
            });
          } catch (error) {
            console.error('Failed to send tool cancellation to backend:', error);
            const { toastError } = await import('../toasts');
            toastError({
              title: t('gooseMessage.failedToCancelTool'),
              msg: t('gooseMessage.agentWaitingResponse'),
            });
          }
        })();
      }
    }
  }, [
    messageIndex,
    messageHistoryIndex,
    hasToolConfirmation,
    toolConfirmationContent,
    messages,
    sessionId,
  ]);

  return (
    <div className="goose-message flex w-[90%] justify-start min-w-0">
      <div className="flex flex-col w-full min-w-0">
        {showThinking && allThinkingText && (
          <ThinkingBlock content={allThinkingText} className="mb-2" isStreaming={isStreaming} />
        )}

        {displayText && (
          <div className="flex flex-col group">
            <div ref={contentRef} className="w-full">
              <MarkdownContent content={displayText} />
            </div>

            {imagePaths.length > 0 && (
              <div className="mt-4">
                {imagePaths.map((imagePath, index) => (
                  <ImagePreview key={index} src={imagePath} />
                ))}
              </div>
            )}

            {toolRequests.length === 0 && (
              <div className="relative flex justify-start">
                {!isStreaming && (
                  <div className="text-xs font-mono text-text-muted pt-1 transition-all duration-200 group-hover:-translate-y-4 group-hover:opacity-0">
                    {timestamp}
                  </div>
                )}
                {message.content.every((content) => content.type === 'text') && !isStreaming && (
                  <div className="absolute left-0 pt-1">
                    <MessageCopyLink text={displayText} contentRef={contentRef} />
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {toolRequests.length > 0 && (
          <div className={cn(displayText && 'mt-2')}>
            <div className="relative flex flex-col w-full">
              <div className="flex flex-col gap-3">
                {toolRequests.map((toolRequest) => (
                  <div className="goose-message-tool" key={toolRequest.id}>
                    <ToolCallWithResponse
                      isCancelledMessage={
                        messageIndex < messageHistoryIndex &&
                        toolResponsesMap.get(toolRequest.id) == undefined
                      }
                      toolRequest={toolRequest}
                      toolResponse={toolResponsesMap.get(toolRequest.id)}
                      notifications={toolCallNotifications.get(toolRequest.id)}
                      isStreamingMessage={isStreaming}
                      append={append}
                    />
                  </div>
                ))}
              </div>
              <div className="text-xs text-text-muted transition-all duration-200 group-hover:-translate-y-4 group-hover:opacity-0 pt-1">
                {!isStreaming && !hideTimestamp && timestamp}
              </div>
            </div>
          </div>
        )}

        {hasToolConfirmation && (
          <ToolCallConfirmation
            sessionId={sessionId}
            isCancelledMessage={messageIndex == messageHistoryIndex - 1}
            isClicked={messageIndex < messageHistoryIndex}
            actionRequiredContent={toolConfirmationContent}
          />
        )}

        {hasElicitation && submitElicitationResponse && (
          <ElicitationRequest
            isCancelledMessage={messageIndex == messageHistoryIndex - 1}
            isClicked={messageIndex < messageHistoryIndex}
            actionRequiredContent={elicitationContent}
            onSubmit={submitElicitationResponse}
          />
        )}
      </div>
    </div>
  );
}
