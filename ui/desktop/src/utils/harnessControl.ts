/**
 * Helpers for interpreting the harness control SSE protocol on the client.
 *
 * The backend emits `HarnessControlEnvelope` frames (see
 * `crates/agime/src/agents/harness/control_types.rs`) wrapping a tagged
 * `payload` of `{ channel, event }`. These arrive in two places:
 *  - inside the chat reply stream as `MessageEvent { type: 'HarnessControl' }`
 *  - inside a task stream as `UserTaskEvent` with a `control` payload
 *
 * Both carry the same envelope shape, so we parse defensively from `unknown`
 * and produce a short human-readable line for display.
 */

/** The `{ channel, event }` payload inside a `HarnessControlEnvelope`. */
interface ControlPayload {
  channel?: string;
  event?: { type?: string; [key: string]: unknown };
}

interface ControlEnvelope {
  sequence?: number;
  payload?: ControlPayload;
}

function asEnvelope(value: unknown): ControlEnvelope | null {
  if (!value || typeof value !== 'object') {
    return null;
  }
  const env = value as ControlEnvelope;
  if (!env.payload || typeof env.payload !== 'object') {
    return null;
  }
  return env;
}

function str(event: Record<string, unknown>, key: string): string | undefined {
  const v = event[key];
  return typeof v === 'string' ? v : undefined;
}

/**
 * Render a single harness control envelope as a short status line, or null if
 * the frame isn't a recognized control envelope (e.g. a plain task lifecycle
 * event). Accepts either a raw envelope object or a `UserTaskEvent` whose
 * `payload.envelope` holds one.
 */
export function describeHarnessEnvelope(input: unknown): string | null {
  // A UserTaskEvent wraps the envelope under payload.envelope when kind=control.
  let raw: unknown = input;
  if (input && typeof input === 'object') {
    const maybe = input as { payload?: { kind?: string; envelope?: unknown } };
    if (maybe.payload?.kind === 'control' && maybe.payload.envelope != null) {
      raw = maybe.payload.envelope;
    }
  }

  const env = asEnvelope(raw);
  if (!env) {
    return null;
  }
  const channel = env.payload!.channel;
  const event = env.payload!.event ?? {};
  const type = event.type;

  switch (channel) {
    case 'session':
      if (type === 'started') return `会话已启动 (${str(event, 'mode') ?? ''})`;
      if (type === 'state_changed') return `状态：${str(event, 'state') ?? ''}`;
      if (type === 'interrupted') return '会话被中断';
      if (type === 'cancel_requested') return '请求取消';
      if (type === 'finished')
        return `会话结束：${str(event, 'final_status') ?? ''}`;
      return `会话事件：${type ?? ''}`;
    case 'tool':
      if (type === 'started') return `工具开始：${str(event, 'tool_name') ?? ''}`;
      if (type === 'progress')
        return `工具进度：${str(event, 'tool_name') ?? ''} ${str(event, 'message') ?? ''}`;
      if (type === 'finished') {
        const ok = event.success === true ? '成功' : '失败';
        return `工具完成：${str(event, 'tool_name') ?? ''} (${ok})`;
      }
      if (type === 'transport_requested')
        return `工具传输请求：${str(event, 'tool_name') ?? ''}`;
      return `工具事件：${type ?? ''}`;
    case 'permission':
      if (type === 'requested')
        return `权限请求：${str(event, 'tool_name') ?? ''}`;
      if (type === 'resolved')
        return `权限已处理：${str(event, 'decision') ?? ''}`;
      if (type === 'timed_out') return '权限请求超时';
      if (type === 'requires_action')
        return `需要操作：${str(event, 'title') ?? ''}`;
      return `权限事件：${type ?? ''}`;
    case 'worker':
      if (type === 'started') return `子任务开始：${str(event, 'kind') ?? ''}`;
      if (type === 'progress')
        return `子任务进度：${str(event, 'message') ?? ''}`;
      if (type === 'idle') return `子任务空闲：${str(event, 'message') ?? ''}`;
      if (type === 'followup_requested')
        return `子任务追加请求：${str(event, 'reason') ?? ''}`;
      if (type === 'finished')
        return `子任务完成：${str(event, 'status') ?? ''} - ${str(event, 'summary') ?? ''}`;
      return `子任务事件：${type ?? ''}`;
    case 'completion':
      if (type === 'structured_published')
        return `结果已发布：${str(event, 'status') ?? ''}`;
      if (type === 'outcome_observed')
        return `结果：${str(event, 'status') ?? ''}`;
      return `完成事件：${type ?? ''}`;
    case 'runtime':
      if (type === 'compaction_observed') return '上下文已压缩';
      if (type === 'notification') return str(event, 'message') ?? '运行时通知';
      return `运行时事件：${type ?? ''}`;
    default:
      return null;
  }
}

/**
 * Extract the `{ channel, type }` discriminator of a chat `HarnessControl`
 * envelope, used by the chat stream manager to surface a compaction hint
 * without mutating the message list. Returns null for unrecognized frames.
 */
export function harnessEnvelopeKind(
  input: unknown
): { channel: string; type: string } | null {
  const env = asEnvelope(input);
  const channel = env?.payload?.channel;
  const type = env?.payload?.event?.type;
  if (typeof channel === 'string' && typeof type === 'string') {
    return { channel, type };
  }
  return null;
}
