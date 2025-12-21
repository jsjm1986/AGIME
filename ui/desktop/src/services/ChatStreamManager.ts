/**
 * ChatStreamManager - Global singleton for managing chat streams
 *
 * This manager persists stream connections independent of React component lifecycle,
 * solving the issue where streams are interrupted when users switch pages.
 *
 * Architecture:
 * - Singleton pattern ensures single source of truth
 * - Subscriber pattern allows components to receive updates
 * - Supports multiple concurrent sessions
 * - Cross-platform compatible (Windows/macOS/Linux)
 */
/* eslint-disable no-undef */

import {
  Message,
  MessageEvent,
  TokenState,
  Session,
  reply,
  resumeAgent,
} from '../api';
import { ChatState } from '../types/chatState';
import {
  NotificationEvent,
  getCompactingMessage,
  getThinkingMessage,
  hasExtendedThinking,
} from '../types/message';
import { errorMessage } from '../utils/conversionUtils';

export interface StreamState {
  messages: Message[];
  session?: Session;
  chatState: ChatState;
  tokenState: TokenState;
  notifications: NotificationEvent[];
  error?: string;
}

export type StreamSubscriber = (state: StreamState) => void;

class ChatStreamManager {
  private static instance: ChatStreamManager;

  // Stream state for each session
  private streams: Map<string, StreamState> = new Map();

  // Subscribers for each session
  private subscribers: Map<string, Set<StreamSubscriber>> = new Map();

  // AbortController for each session
  private abortControllers: Map<string, AbortController> = new Map();

  // Cache for loaded sessions
  private sessionCache: Map<string, { session: Session; messages: Message[] }> =
    new Map();

  private constructor() {}

  static getInstance(): ChatStreamManager {
    if (!ChatStreamManager.instance) {
      ChatStreamManager.instance = new ChatStreamManager();
    }
    return ChatStreamManager.instance;
  }

  /**
   * Subscribe to state updates for a specific session
   * @returns Unsubscribe function
   */
  subscribe(sessionId: string, callback: StreamSubscriber): () => void {
    if (!this.subscribers.has(sessionId)) {
      this.subscribers.set(sessionId, new Set());
    }
    this.subscribers.get(sessionId)!.add(callback);

    // Immediately send current state if available
    const currentState = this.streams.get(sessionId);
    if (currentState) {
      callback(currentState);
    }

    // Return unsubscribe function
    return () => {
      this.subscribers.get(sessionId)?.delete(callback);
      if (this.subscribers.get(sessionId)?.size === 0) {
        this.subscribers.delete(sessionId);
      }
    };
  }

  /**
   * Get current state for a session
   */
  getState(sessionId: string): StreamState | undefined {
    return this.streams.get(sessionId);
  }

  /**
   * Get cached session data
   */
  getCachedSession(
    sessionId: string
  ): { session: Session; messages: Message[] } | undefined {
    return this.sessionCache.get(sessionId);
  }

  /**
   * Update cache with session data
   */
  updateCache(sessionId: string, session: Session, messages: Message[]): void {
    this.sessionCache.set(sessionId, { session, messages });
  }

  /**
   * Initialize or resume a session
   */
  async initializeSession(sessionId: string): Promise<Session> {
    const response = await resumeAgent({
      body: {
        session_id: sessionId,
        load_model_and_extensions: true,
      },
      throwOnError: true,
    });

    const session = response.data;
    const messages = session?.conversation || [];

    // Update cache
    if (session) {
      this.updateCache(sessionId, session, messages);
    }

    // Initialize state if no active stream, or update session for active stream
    const currentState = this.streams.get(sessionId);
    if (!currentState || currentState.chatState === ChatState.Idle) {
      this.updateState(sessionId, {
        messages,
        session,
        chatState: ChatState.Idle,
        tokenState: this.getDefaultTokenState(),
        notifications: [],
      });
    } else {
      // If there's an active stream, keep the stream state but update session
      // and merge messages (use the longer list)
      this.updateState(sessionId, {
        ...currentState,
        session,
        messages:
          messages.length > currentState.messages.length
            ? messages
            : currentState.messages,
      });
    }

    return session;
  }

  /**
   * Start a new stream for a session
   */
  async startStream(sessionId: string, messages: Message[]): Promise<void> {
    const streamStart = performance.now();
    console.log('[PERF] startStream called', streamStart);

    // Abort previous stream if exists
    this.stopStream(sessionId);

    const abortController = new AbortController();
    this.abortControllers.set(sessionId, abortController);

    // Initialize state
    const currentState = this.streams.get(sessionId);
    this.updateState(sessionId, {
      messages,
      session: currentState?.session,
      chatState: ChatState.Streaming,
      tokenState: currentState?.tokenState || this.getDefaultTokenState(),
      notifications: [],
    });

    try {
      const { stream } = await reply({
        body: {
          session_id: sessionId,
          messages,
        },
        throwOnError: true,
        signal: abortController.signal,
      });
      console.log('[PERF] reply() API response received', performance.now() - streamStart, 'ms');

      await this.processStream(sessionId, stream, messages, streamStart);
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        // User stopped intentionally, don't report error
        return;
      }
      this.handleError(sessionId, 'Submit error: ' + errorMessage(error));
    }
  }

  /**
   * Stop the stream for a session
   */
  stopStream(sessionId: string): void {
    const controller = this.abortControllers.get(sessionId);
    if (controller) {
      controller.abort();
      this.abortControllers.delete(sessionId);
    }

    const currentState = this.streams.get(sessionId);
    if (currentState && currentState.chatState !== ChatState.Idle) {
      this.updateState(sessionId, {
        ...currentState,
        chatState: ChatState.Idle,
      });
    }
  }

  /**
   * Check if there's an active stream for a session
   */
  isStreamActive(sessionId: string): boolean {
    const state = this.streams.get(sessionId);
    return (
      state !== undefined &&
      state.chatState !== ChatState.Idle &&
      state.chatState !== ChatState.LoadingConversation
    );
  }

  /**
   * Process stream events
   */
  private async processStream(
    sessionId: string,
    stream: AsyncIterable<MessageEvent>,
    initialMessages: Message[],
    streamStart?: number
  ): Promise<void> {
    let currentMessages = initialMessages;
    let firstEventReceived = false;

    try {
      for await (const event of stream) {
        // Log first event timing
        if (!firstEventReceived && streamStart) {
          console.log('[PERF] first stream event received', performance.now() - streamStart, 'ms');
          firstEventReceived = true;
        }

        // Check if aborted
        if (!this.abortControllers.has(sessionId)) {
          return;
        }

        switch (event.type) {
          case 'Message': {
            currentMessages = this.pushMessage(currentMessages, event.message);

            let newChatState = ChatState.Streaming;
            const hasToolConfirmation = event.message.content.some(
              (c) => c.type === 'toolConfirmationRequest'
            );
            const hasElicitation = event.message.content.some(
              (c) =>
                c.type === 'actionRequired' &&
                c.data.actionType === 'elicitation'
            );

            if (hasToolConfirmation || hasElicitation) {
              newChatState = ChatState.WaitingForUserInput;
            } else if (getCompactingMessage(event.message)) {
              newChatState = ChatState.Compacting;
            } else if (getThinkingMessage(event.message) || hasExtendedThinking(event.message)) {
              // Check for both systemNotification thinking messages and Claude Extended Thinking
              newChatState = ChatState.Thinking;
            }

            const currentState = this.streams.get(sessionId)!;
            this.updateState(sessionId, {
              ...currentState,
              messages: currentMessages,
              chatState: newChatState,
              tokenState: event.token_state,
            });
            break;
          }
          case 'Error': {
            this.handleError(sessionId, 'Stream error: ' + event.error);
            return;
          }
          case 'Finish': {
            this.handleFinish(sessionId);
            return;
          }
          case 'UpdateConversation': {
            currentMessages = event.conversation;
            const currentState = this.streams.get(sessionId)!;
            this.updateState(sessionId, {
              ...currentState,
              messages: currentMessages,
            });
            break;
          }
          case 'Notification': {
            const currentState = this.streams.get(sessionId)!;
            this.updateState(sessionId, {
              ...currentState,
              notifications: [
                ...currentState.notifications,
                event as NotificationEvent,
              ],
            });
            break;
          }
          case 'ModelChange':
          case 'Ping':
            break;
        }
      }

      this.handleFinish(sessionId);
    } catch (error) {
      if (error instanceof Error && error.name !== 'AbortError') {
        this.handleError(sessionId, 'Stream error: ' + errorMessage(error));
      }
    }
  }

  /**
   * Merge incoming message with current messages
   * Supports incremental merging for both text and thinking content
   */
  private pushMessage(
    currentMessages: Message[],
    incomingMsg: Message
  ): Message[] {
    const lastMsg = currentMessages[currentMessages.length - 1];

    if (lastMsg?.id && lastMsg.id === incomingMsg.id) {
      const lastContent = lastMsg.content[lastMsg.content.length - 1];
      const newContent = incomingMsg.content[incomingMsg.content.length - 1];

      // Handle text content merging (streaming text)
      if (
        lastContent?.type === 'text' &&
        newContent?.type === 'text' &&
        incomingMsg.content.length === 1
      ) {
        lastContent.text += newContent.text;
      }
      // Handle thinking content merging (streaming thinking)
      else if (
        lastContent?.type === 'thinking' &&
        newContent?.type === 'thinking' &&
        incomingMsg.content.length === 1
      ) {
        // Append thinking content for real-time streaming display
        (lastContent as { thinking: string; signature: string }).thinking +=
          (newContent as { thinking: string; signature: string }).thinking;
        // Update signature if provided (usually at the end)
        const newSignature = (newContent as { thinking: string; signature: string }).signature;
        if (newSignature) {
          (lastContent as { thinking: string; signature: string }).signature = newSignature;
        }
      } else {
        lastMsg.content.push(...incomingMsg.content);
      }
      return [...currentMessages];
    } else {
      return [...currentMessages, incomingMsg];
    }
  }

  /**
   * Handle stream finish
   */
  private handleFinish(sessionId: string): void {
    const currentState = this.streams.get(sessionId);
    if (currentState) {
      this.updateState(sessionId, {
        ...currentState,
        chatState: ChatState.Idle,
      });

      // Update cache
      if (currentState.session) {
        this.updateCache(
          sessionId,
          currentState.session,
          currentState.messages
        );
      }
    }

    this.abortControllers.delete(sessionId);

    // Check if this is a new session, dispatch event
    const isNewSession = sessionId.match(/^\d{8}_\d{6}$/);
    if (isNewSession) {
      window.dispatchEvent(new CustomEvent('message-stream-finished'));
    }
  }

  /**
   * Handle stream error
   */
  private handleError(sessionId: string, error: string): void {
    const currentState = this.streams.get(sessionId);
    this.updateState(sessionId, {
      ...(currentState || this.getDefaultState()),
      chatState: ChatState.Idle,
      error,
    });
    this.abortControllers.delete(sessionId);
  }

  /**
   * Update state and notify subscribers
   */
  private updateState(sessionId: string, state: StreamState): void {
    this.streams.set(sessionId, state);
    this.notifySubscribers(sessionId, state);
  }

  /**
   * Notify all subscribers for a session
   */
  private notifySubscribers(sessionId: string, state: StreamState): void {
    const subscribers = this.subscribers.get(sessionId);
    if (subscribers) {
      subscribers.forEach((callback) => {
        try {
          callback(state);
        } catch (e) {
          console.error('Error in stream subscriber:', e);
        }
      });
    }
  }

  /**
   * Get default TokenState
   */
  private getDefaultTokenState(): TokenState {
    return {
      inputTokens: 0,
      outputTokens: 0,
      totalTokens: 0,
      accumulatedInputTokens: 0,
      accumulatedOutputTokens: 0,
      accumulatedTotalTokens: 0,
    };
  }

  /**
   * Get default StreamState
   */
  private getDefaultState(): StreamState {
    return {
      messages: [],
      chatState: ChatState.Idle,
      tokenState: this.getDefaultTokenState(),
      notifications: [],
    };
  }

  /**
   * Cleanup all state for a session
   */
  cleanup(sessionId: string): void {
    this.stopStream(sessionId);
    this.streams.delete(sessionId);
    this.subscribers.delete(sessionId);
    this.sessionCache.delete(sessionId);
  }
}

export const chatStreamManager = ChatStreamManager.getInstance();
