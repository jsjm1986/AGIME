import { describe, it, expect } from 'vitest';
import {
  describeHarnessEnvelope,
  harnessEnvelopeKind,
} from './harnessControl';

function envelope(channel: string, event: Record<string, unknown>) {
  return {
    session_id: 'logical-1',
    runtime_session_id: 'runtime-1',
    sequence: 1,
    timestamp_ms: 0,
    payload: { channel, event },
  };
}

describe('harnessControl', () => {
  describe('describeHarnessEnvelope', () => {
    it('describes a tool started event', () => {
      const line = describeHarnessEnvelope(
        envelope('tool', { type: 'started', tool_name: 'shell' })
      );
      expect(line).toBe('工具开始：shell');
    });

    it('describes a tool finished event with success flag', () => {
      expect(
        describeHarnessEnvelope(
          envelope('tool', { type: 'finished', tool_name: 'shell', success: true })
        )
      ).toBe('工具完成：shell (成功)');
      expect(
        describeHarnessEnvelope(
          envelope('tool', { type: 'finished', tool_name: 'shell', success: false })
        )
      ).toBe('工具完成：shell (失败)');
    });

    it('describes a runtime compaction event', () => {
      expect(
        describeHarnessEnvelope(
          envelope('runtime', { type: 'compaction_observed' })
        )
      ).toBe('上下文已压缩');
    });

    it('describes a worker finished event', () => {
      expect(
        describeHarnessEnvelope(
          envelope('worker', {
            type: 'finished',
            status: 'completed',
            summary: 'done',
          })
        )
      ).toBe('子任务完成：completed - done');
    });

    it('unwraps a UserTaskEvent control payload', () => {
      const taskEvent = {
        task_id: 't1',
        seq: 5,
        payload: {
          kind: 'control',
          envelope: envelope('permission', {
            type: 'requested',
            tool_name: 'shell',
          }),
        },
      };
      expect(describeHarnessEnvelope(taskEvent)).toBe('权限请求：shell');
    });

    it('returns null for unrecognized input', () => {
      expect(describeHarnessEnvelope(null)).toBeNull();
      expect(describeHarnessEnvelope({})).toBeNull();
      expect(describeHarnessEnvelope({ payload: {} })).toBeNull();
      expect(
        describeHarnessEnvelope(envelope('unknown_channel', { type: 'x' }))
      ).toBeNull();
    });
  });

  describe('harnessEnvelopeKind', () => {
    it('extracts channel and type', () => {
      expect(
        harnessEnvelopeKind(
          envelope('runtime', { type: 'compaction_observed' })
        )
      ).toEqual({ channel: 'runtime', type: 'compaction_observed' });
    });

    it('returns null when channel or type is missing', () => {
      expect(harnessEnvelopeKind(envelope('runtime', {}))).toBeNull();
      expect(harnessEnvelopeKind({ payload: { event: { type: 'x' } } })).toBeNull();
      expect(harnessEnvelopeKind(undefined)).toBeNull();
    });
  });
});
