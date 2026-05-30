import { describe, it, expect, vi, beforeEach } from 'vitest';
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

import { chatStreamManager, StreamState } from './ChatStreamManager';

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
