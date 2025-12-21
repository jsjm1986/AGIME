/**
 * useChatStream - React hook for chat streaming
 *
 * This hook now uses ChatStreamManager for stream management,
 * which persists stream connections independent of component lifecycle.
 * This solves the issue where streams are interrupted when users switch pages.
 */
/* eslint-disable no-undef */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChatState } from '../types/chatState';

import {
  Message,
  Session,
  TokenState,
  updateFromSession,
  updateSessionUserRecipeValues,
} from '../api';

import {
  createUserMessage,
  createElicitationResponseMessage,
  NotificationEvent,
} from '../types/message';
import { errorMessage } from '../utils/conversionUtils';
import {
  chatStreamManager,
  StreamState,
} from '../services/ChatStreamManager';

interface UseChatStreamProps {
  sessionId: string;
  onStreamFinish: () => void;
  onSessionLoaded?: () => void;
}

interface UseChatStreamReturn {
  session?: Session;
  messages: Message[];
  chatState: ChatState;
  handleSubmit: (userMessage: string) => Promise<void>;
  submitElicitationResponse: (
    elicitationId: string,
    userData: Record<string, unknown>
  ) => Promise<void>;
  setRecipeUserParams: (values: Record<string, string>) => Promise<void>;
  stopStreaming: () => void;
  sessionLoadError?: string;
  tokenState: TokenState;
  notifications: Map<string, NotificationEvent[]>;
  onMessageUpdate: (
    messageId: string,
    newContent: string,
    editType?: 'fork' | 'edit'
  ) => Promise<void>;
}

const DEFAULT_TOKEN_STATE: TokenState = {
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
};

export function useChatStream({
  sessionId,
  onStreamFinish,
  onSessionLoaded,
}: UseChatStreamProps): UseChatStreamReturn {
  // Initialize state from cache or manager
  const [state, setState] = useState<StreamState>(() => {
    const cached = chatStreamManager.getCachedSession(sessionId);
    const existingState = chatStreamManager.getState(sessionId);

    return (
      existingState || {
        messages: cached?.messages || [],
        session: cached?.session,
        chatState: ChatState.Idle,
        tokenState: DEFAULT_TOKEN_STATE,
        notifications: [],
      }
    );
  });

  const [sessionLoadError, setSessionLoadError] = useState<string>();
  const messagesRef = useRef<Message[]>(state.messages);

  // Keep messagesRef in sync
  useEffect(() => {
    messagesRef.current = state.messages;
  }, [state.messages]);

  // Subscribe to ChatStreamManager
  useEffect(() => {
    if (!sessionId) return;

    // Subscribe to state updates
    const unsubscribe = chatStreamManager.subscribe(sessionId, (newState) => {
      const updateStart = performance.now();
      console.log('[PERF] state update received, messages:', newState.messages.length);
      setState(newState);
      console.log('[PERF] setState done in', performance.now() - updateStart, 'ms');
      messagesRef.current = newState.messages;

      if (newState.error) {
        setSessionLoadError(newState.error);
      }
    });

    // Check if we need to initialize the session
    const isActive = chatStreamManager.isStreamActive(sessionId);
    const existingState = chatStreamManager.getState(sessionId);
    const cachedSession = chatStreamManager.getCachedSession(sessionId);

    if (!isActive) {
      // If we have cached data with a valid session, use it without API call
      if (cachedSession?.session && cachedSession.messages) {
        setState({
          messages: cachedSession.messages,
          session: cachedSession.session,
          chatState: ChatState.Idle,
          tokenState: DEFAULT_TOKEN_STATE,
          notifications: [],
        });
        onSessionLoaded?.();
      } else if (!existingState?.session) {
        // No cache and no existing state - need to load from API
        setState((prev) => ({
          ...prev,
          chatState: ChatState.LoadingConversation,
        }));

        chatStreamManager
          .initializeSession(sessionId)
          .then(() => {
            onSessionLoaded?.();
          })
          .catch((error) => {
            setSessionLoadError(errorMessage(error));
            setState((prev) => ({
              ...prev,
              chatState: ChatState.Idle,
            }));
          });
      } else {
        // We have existing state, just use it
        onSessionLoaded?.();
      }
    }

    return unsubscribe;
  }, [sessionId, onSessionLoaded]);

  // Track chatState changes to trigger onStreamFinish
  const prevChatStateRef = useRef(state.chatState);
  useEffect(() => {
    const wasActive =
      prevChatStateRef.current !== ChatState.Idle &&
      prevChatStateRef.current !== ChatState.LoadingConversation;
    const isNowIdle = state.chatState === ChatState.Idle;

    if (wasActive && isNowIdle) {
      onStreamFinish();
    }
    prevChatStateRef.current = state.chatState;
  }, [state.chatState, onStreamFinish]);

  // Report chat busy state to main process for close confirmation
  useEffect(() => {
    const isBusy =
      state.chatState !== ChatState.Idle &&
      state.chatState !== ChatState.LoadingConversation;
    window.electron?.setChatBusy?.(isBusy);
  }, [state.chatState]);

  const handleSubmit = useCallback(
    async (userMessage: string) => {
      // Guard: Don't submit if session hasn't been loaded yet
      if (!state.session || state.chatState === ChatState.LoadingConversation) {
        return;
      }

      const hasExistingMessages = messagesRef.current.length > 0;
      const hasNewMessage = userMessage.trim().length > 0;

      // Don't submit if there's no message and no conversation to continue
      if (!hasNewMessage && !hasExistingMessages) {
        return;
      }

      // Emit session-created event for first message in a new session
      if (!hasExistingMessages && hasNewMessage) {
        window.dispatchEvent(new CustomEvent('session-created'));
      }

      // Build message list: add new message if provided, otherwise continue with existing
      const currentMessages = hasNewMessage
        ? [...messagesRef.current, createUserMessage(userMessage)]
        : [...messagesRef.current];

      await chatStreamManager.startStream(sessionId, currentMessages);
    },
    [sessionId, state.session, state.chatState]
  );

  const submitElicitationResponse = useCallback(
    async (elicitationId: string, userData: Record<string, unknown>) => {
      if (!state.session || state.chatState === ChatState.LoadingConversation) {
        return;
      }

      const responseMessage = createElicitationResponseMessage(
        elicitationId,
        userData
      );
      const currentMessages = [...messagesRef.current, responseMessage];

      await chatStreamManager.startStream(sessionId, currentMessages);
    },
    [sessionId, state.session, state.chatState]
  );

  const setRecipeUserParams = useCallback(
    async (user_recipe_values: Record<string, string>) => {
      if (state.session) {
        await updateSessionUserRecipeValues({
          path: {
            session_id: sessionId,
          },
          body: {
            userRecipeValues: user_recipe_values,
          },
          throwOnError: true,
        });
        // Update local state
        setState((prev) => ({
          ...prev,
          session: prev.session
            ? { ...prev.session, user_recipe_values }
            : undefined,
        }));
      } else {
        setSessionLoadError("can't call setRecipeParams without a session");
      }
    },
    [sessionId, state.session]
  );

  useEffect(() => {
    // This should happen on the server when the session is loaded or changed
    // use session.id to support changing of sessions rather than depending on the
    // stable sessionId.
    if (state.session) {
      updateFromSession({
        body: {
          session_id: state.session.id,
        },
        throwOnError: true,
      });
    }
  }, [state.session]);

  const stopStreaming = useCallback(() => {
    chatStreamManager.stopStream(sessionId);
  }, [sessionId]);

  const onMessageUpdate = useCallback(
    async (
      messageId: string,
      newContent: string,
      editType: 'fork' | 'edit' = 'fork'
    ) => {
      try {
        const { editMessage } = await import('../api');
        const message = messagesRef.current.find((m) => m.id === messageId);

        if (!message) {
          throw new Error(
            `Message with id ${messageId} not found in current messages`
          );
        }

        const response = await editMessage({
          path: {
            session_id: sessionId,
          },
          body: {
            timestamp: message.created,
            editType,
          },
          throwOnError: true,
        });

        const targetSessionId = response.data?.sessionId;
        if (!targetSessionId) {
          throw new Error('No session ID returned from edit_message');
        }

        if (editType === 'fork') {
          const event = new CustomEvent('session-forked', {
            detail: {
              newSessionId: targetSessionId,
              shouldStartAgent: true,
              editedMessage: newContent,
            },
          });
          window.dispatchEvent(event);
          window.electron.logInfo(
            `Dispatched session-forked event for session ${targetSessionId}`
          );
        } else {
          const { getSession } = await import('../api');
          const sessionResponse = await getSession({
            path: { session_id: targetSessionId },
            throwOnError: true,
          });

          if (sessionResponse.data?.conversation) {
            // Update via global manager
            chatStreamManager.updateCache(
              sessionId,
              sessionResponse.data,
              sessionResponse.data.conversation
            );
            setState((prev) => ({
              ...prev,
              messages: sessionResponse.data.conversation || [],
              session: sessionResponse.data,
            }));
          }
          await handleSubmit(newContent);
        }
      } catch (error) {
        const errorMsg = errorMessage(error);
        console.error('Failed to edit message:', error);
        const { toastError } = await import('../toasts');
        toastError({
          title: 'Failed to edit message',
          msg: errorMsg,
        });
      }
    },
    [sessionId, handleSubmit]
  );

  // Convert notifications array to Map
  const notificationsMap = useMemo(() => {
    return state.notifications.reduce((map, notification) => {
      const key = notification.request_id;
      if (!map.has(key)) {
        map.set(key, []);
      }
      map.get(key)!.push(notification);
      return map;
    }, new Map<string, NotificationEvent[]>());
  }, [state.notifications]);

  return {
    sessionLoadError,
    messages: state.messages,
    session: state.session,
    chatState: state.chatState,
    handleSubmit,
    submitElicitationResponse,
    stopStreaming,
    setRecipeUserParams,
    tokenState: state.tokenState,
    notifications: notificationsMap,
    onMessageUpdate,
  };
}
