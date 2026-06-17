import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { Message, MessageEvent } from '../api';

const replyMock = vi.fn();
const resumeAgentMock = vi.fn();

vi.mock('../api', () => ({
  reply: (...args: unknown[]) => replyMock(...args),
  resumeAgent: (...args: unknown[]) => resumeAgentMock(...args),
}));

vi.mock('react-toastify', () => ({
  toast: { error: vi.fn() },
}));

import { toast } from 'react-toastify';
import { chatStreamManager, StreamState } from './ChatStreamManager';
import { ChatState } from '../types/chatState';


function controlEnvelope(channel: string, event: Record<string, unknown>) {
  return {
    session_id: 'logical-1',
    runtime_session_id: 'runtime-1',
    sequence: 1,
    timestamp_ms: 0,
    payload: { channel, event },
  };
}

function textMessage(id: string, text: string): Message {
  return {
    id,
    role: 'assistant',
    created: 0,
    content: [{ type: 'text', text }],
  } as unknown as Message;
}

function streamOf(events: MessageEvent[]): AsyncIterable<MessageEvent> {
  return {
    async *[Symbol.asyncIterator]() {
      for (const e of events) {
        yield e;
      }
    },
  };
}

async function runStream(
  sessionId: string,
  events: MessageEvent[]
): Promise<StreamState[]> {
  const captured: StreamState[] = [];
  const unsubscribe = chatStreamManager.subscribe(sessionId, (s) => {
    captured.push(JSON.parse(JSON.stringify(s)) as StreamState);
  });
  replyMock.mockResolvedValueOnce({ stream: streamOf(events) });
  await chatStreamManager.startStream(sessionId, []);
  unsubscribe();
  return captured;
}

describe('ChatStreamManager', () => {
  beforeEach(() => {
    replyMock.mockReset();
    resumeAgentMock.mockReset();
  });

  it('surfaces a harness control status line', async () => {
    const sessionId = 'sess-harness';
    chatStreamManager.cleanup(sessionId);

    const states = await runStream(sessionId, [
      {
        type: 'HarnessControl',
        envelope: controlEnvelope('worker', {
          type: 'started',
          kind: 'swarm',
        }),
      } as unknown as MessageEvent,
    ]);

    const sawStatus = states.some(
      (s) => s.harnessStatus === '子任务开始：swarm'
    );
    expect(sawStatus).toBe(true);
  });

  it('clears the harness status when the session finishes', async () => {
    const sessionId = 'sess-harness-end';
    chatStreamManager.cleanup(sessionId);

    const states = await runStream(sessionId, [
      {
        type: 'HarnessControl',
        envelope: controlEnvelope('worker', { type: 'started', kind: 'swarm' }),
      } as unknown as MessageEvent,
      {
        type: 'HarnessControl',
        envelope: controlEnvelope('session', {
          type: 'finished',
          final_status: 'completed',
        }),
      } as unknown as MessageEvent,
    ]);

    expect(states[states.length - 1].harnessStatus).toBeUndefined();
  });

  it('ignores an empty UpdateConversation when messages already exist', async () => {
    const sessionId = 'sess-empty-update';
    chatStreamManager.cleanup(sessionId);

    await runStream(sessionId, [
      {
        type: 'Message',
        message: textMessage('m1', 'hello'),
        token_state: undefined,
      } as unknown as MessageEvent,
      {
        type: 'UpdateConversation',
        conversation: [],
      } as unknown as MessageEvent,
    ]);

    const finalState = chatStreamManager.getState(sessionId);
    expect(finalState?.messages.length).toBe(1);
    expect(finalState?.messages[0].id).toBe('m1');
  });

  it('applies a non-empty UpdateConversation replacement', async () => {
    const sessionId = 'sess-replace-update';
    chatStreamManager.cleanup(sessionId);

    await runStream(sessionId, [
      {
        type: 'Message',
        message: textMessage('m1', 'hello'),
        token_state: undefined,
      } as unknown as MessageEvent,
      {
        type: 'UpdateConversation',
        conversation: [textMessage('compacted', 'summary')],
      } as unknown as MessageEvent,
    ]);

    const finalState = chatStreamManager.getState(sessionId);
    expect(finalState?.messages.length).toBe(1);
    expect(finalState?.messages[0].id).toBe('compacted');
  });
});

describe('ChatStreamManager stream timeouts', () => {
  beforeEach(() => {
    replyMock.mockReset();
    resumeAgentMock.mockReset();
    (toast.error as ReturnType<typeof vi.fn>).mockClear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // A stream that yields the provided events then hangs forever on its next
  // pull — models a provider that accepts the request and then stalls.
  function hangingStreamAfter(events: MessageEvent[]): AsyncIterable<MessageEvent> {
    return {
      async *[Symbol.asyncIterator]() {
        for (const e of events) {
          yield e;
        }
        await new Promise<never>(() => {});
      },
    };
  }

  function startHangingStream(sessionId: string, events: MessageEvent[]) {
    chatStreamManager.cleanup(sessionId);
    let capturedSignal: AbortSignal | undefined;
    replyMock.mockImplementationOnce((opts: { signal?: AbortSignal }) => {
      capturedSignal = opts.signal;
      return Promise.resolve({ stream: hangingStreamAfter(events) });
    });
    // Do not await: startStream resolves only once the stream ends, which a
    // hanging stream never does without the timeout firing.
    void chatStreamManager.startStream(sessionId, []);
    return () => capturedSignal;
  }

  it('aborts and surfaces an error when no first byte arrives', async () => {
    const sessionId = 'sess-first-byte-timeout';
    const getSignal = startHangingStream(sessionId, []);

    // Let startStream reach the for-await and arm the first-byte timer.
    await vi.advanceTimersByTimeAsync(60_000);

    expect(getSignal()?.aborted).toBe(true);
    expect(toast.error).toHaveBeenCalledTimes(1);
    const finalState = chatStreamManager.getState(sessionId);
    expect(finalState?.chatState).toBe(ChatState.Idle);
    expect(finalState?.error).toBeTruthy();
  });

  it('aborts on idle timeout after frames stop flowing', async () => {
    const sessionId = 'sess-idle-timeout';
    const getSignal = startHangingStream(sessionId, [
      {
        type: 'Message',
        message: textMessage('m1', 'partial'),
        token_state: undefined,
      } as unknown as MessageEvent,
    ]);

    // The single frame resets the watchdog to the idle window; advance past it.
    await vi.advanceTimersByTimeAsync(120_000);

    expect(getSignal()?.aborted).toBe(true);
    expect(toast.error).toHaveBeenCalledTimes(1);
    expect(chatStreamManager.getState(sessionId)?.chatState).toBe(ChatState.Idle);
  });

  it('does not fire a timeout for a normally completing stream', async () => {
    const sessionId = 'sess-no-false-timeout';
    chatStreamManager.cleanup(sessionId);
    replyMock.mockResolvedValueOnce({
      stream: streamOf([
        {
          type: 'Message',
          message: textMessage('m1', 'hi'),
          token_state: undefined,
        } as unknown as MessageEvent,
        { type: 'Finish' } as unknown as MessageEvent,
      ]),
    });

    await chatStreamManager.startStream(sessionId, []);
    // Advance well past both timeout windows; nothing should fire.
    await vi.advanceTimersByTimeAsync(200_000);

    expect(toast.error).not.toHaveBeenCalled();
    expect(chatStreamManager.getState(sessionId)?.chatState).toBe(ChatState.Idle);
  });

  it('passes a bounded sseMaxRetryAttempts to reply', async () => {
    const sessionId = 'sess-retry-bound';
    chatStreamManager.cleanup(sessionId);
    replyMock.mockResolvedValueOnce({
      stream: streamOf([{ type: 'Finish' } as unknown as MessageEvent]),
    });

    await chatStreamManager.startStream(sessionId, []);

    const callArgs = replyMock.mock.calls[0][0] as {
      sseMaxRetryAttempts?: number;
    };
    expect(typeof callArgs.sseMaxRetryAttempts).toBe('number');
    expect(callArgs.sseMaxRetryAttempts).toBeGreaterThan(0);
  });
});
