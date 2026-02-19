import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, SystemNotificationContent } from '../../api';

interface SystemNotificationInlineProps {
  message: Message;
}

const CFPM_RUNTIME_PREFIX = '[CFPM_RUNTIME_V1]';
const CFPM_TOOL_GATE_PREFIX = '[CFPM_TOOL_GATE_V1]';

interface CfpmRuntimeNotificationPayload {
  version: string;
  verbosity: 'off' | 'brief' | 'debug' | string;
  reason: string;
  mode: string;
  acceptedCount: number;
  rejectedCount: number;
  prunedCount: number;
  factCount: number;
  rejectedReasonBreakdown?: string[];
}

interface CfpmToolGateNotificationPayload {
  version: string;
  verbosity: 'off' | 'brief' | 'debug' | string;
  action: string;
  tool: string;
  target: string;
  path: string;
  originalCommand?: string;
  rewrittenCommand?: string;
}

function parsePrefixedJson<T>(rawMessage: string, prefix: string, transform: (parsed: Record<string, unknown>) => T): T | null {
  const trimmed = rawMessage.trim();
  if (!trimmed.startsWith(prefix)) return null;
  const payloadJson = trimmed.slice(prefix.length).trim();
  if (!payloadJson) return null;
  try {
    const parsed = JSON.parse(payloadJson);
    if (!parsed || typeof parsed !== 'object') return null;
    return transform(parsed);
  } catch {
    return null;
  }
}

function parseCfpmRuntimePayload(rawMessage: string): CfpmRuntimeNotificationPayload | null {
  return parsePrefixedJson(rawMessage, CFPM_RUNTIME_PREFIX, (parsed) => ({
    version: String(parsed.version ?? 'v1'),
    verbosity: String(parsed.verbosity ?? 'brief'),
    reason: String(parsed.reason ?? 'unknown'),
    mode: String(parsed.mode ?? 'unknown'),
    acceptedCount: Number(parsed.acceptedCount ?? 0),
    rejectedCount: Number(parsed.rejectedCount ?? 0),
    prunedCount: Number(parsed.prunedCount ?? 0),
    factCount: Number(parsed.factCount ?? 0),
    rejectedReasonBreakdown: Array.isArray(parsed.rejectedReasonBreakdown)
      ? parsed.rejectedReasonBreakdown.map((item: unknown) => String(item))
      : [],
  }));
}

function parseCfpmToolGatePayload(rawMessage: string): CfpmToolGateNotificationPayload | null {
  return parsePrefixedJson(rawMessage, CFPM_TOOL_GATE_PREFIX, (parsed) => ({
    version: String(parsed.version ?? 'v1'),
    verbosity: String(parsed.verbosity ?? 'brief'),
    action: String(parsed.action ?? 'unknown'),
    tool: String(parsed.tool ?? 'unknown'),
    target: String(parsed.target ?? 'unknown'),
    path: String(parsed.path ?? ''),
    originalCommand: parsed.originalCommand == null ? undefined : String(parsed.originalCommand),
    rewrittenCommand: parsed.rewrittenCommand == null ? undefined : String(parsed.rewrittenCommand),
  }));
}

export const SystemNotificationInline: React.FC<SystemNotificationInlineProps> = ({ message }) => {
  const { t } = useTranslation('chat');
  const [showDebugDetails, setShowDebugDetails] = useState(false);

  const systemNotification = message.content.find(
    (content): content is SystemNotificationContent & { type: 'systemNotification' } =>
      content.type === 'systemNotification' && content.notificationType === 'inlineMessage'
  );

  const inlineMessage = systemNotification?.msg ?? '';

  const cfpmRuntimePayload = useMemo(
    () => parseCfpmRuntimePayload(inlineMessage),
    [inlineMessage]
  );
  const cfpmToolGatePayload = useMemo(
    () => parseCfpmToolGatePayload(inlineMessage),
    [inlineMessage]
  );

  if (!inlineMessage) {
    return null;
  }

  if (!cfpmRuntimePayload && !cfpmToolGatePayload) {
    return <div className="text-xs text-gray-400 py-2 text-left">{inlineMessage}</div>;
  }

  if (cfpmRuntimePayload) {
    const isDebug = cfpmRuntimePayload.verbosity === 'debug';
    const rejectedReasons = cfpmRuntimePayload.rejectedReasonBreakdown ?? [];
    const hasDebugReasons = rejectedReasons.length > 0;

    return (
      <div className="py-2 text-left">
        <div className="inline-flex max-w-full flex-col gap-1 rounded border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200">
          <span>
            {t('systemNotification.cfpmSummary', {
              accepted: cfpmRuntimePayload.acceptedCount,
              rejected: cfpmRuntimePayload.rejectedCount,
              pruned: cfpmRuntimePayload.prunedCount,
              facts: cfpmRuntimePayload.factCount,
            })}
          </span>

          {isDebug && (
            <div className="flex items-center justify-between gap-2">
              <span className="text-[11px] opacity-90">
                {t('systemNotification.cfpmDebugMeta', {
                  reason: cfpmRuntimePayload.reason,
                  mode: cfpmRuntimePayload.mode,
                })}
              </span>
              <button
                type="button"
                className="rounded border border-emerald-400/40 px-1.5 py-0.5 text-[11px] hover:bg-emerald-400/10"
                onClick={() => setShowDebugDetails((prev) => !prev)}
              >
                {showDebugDetails
                  ? t('systemNotification.cfpmHideDetails')
                  : t('systemNotification.cfpmShowDetails')}
              </button>
            </div>
          )}

          {isDebug && showDebugDetails && (
            <div className="rounded border border-emerald-500/20 bg-black/15 px-2 py-1 text-[11px]">
              <div>
                {t('systemNotification.cfpmRejectedReasons')}{' '}
                {hasDebugReasons
                  ? rejectedReasons.join(', ')
                  : t('systemNotification.cfpmNoRejectedReasons')}
              </div>
            </div>
          )}
        </div>
      </div>
    );
  }

  const isDebug = cfpmToolGatePayload?.verbosity === 'debug';
  const hasCommandDiff =
    !!cfpmToolGatePayload?.originalCommand && !!cfpmToolGatePayload?.rewrittenCommand;

  return (
    <div className="py-2 text-left">
      <div className="inline-flex max-w-full flex-col gap-1 rounded border border-cyan-500/30 bg-cyan-500/10 px-2 py-1 text-xs text-cyan-200">
        <span>
          {t('systemNotification.cfpmToolGateSummary', {
            tool: cfpmToolGatePayload?.tool ?? 'unknown',
            target: cfpmToolGatePayload?.target ?? 'unknown',
            path: cfpmToolGatePayload?.path ?? '',
          })}
        </span>

        {isDebug && (
          <div className="flex items-center justify-between gap-2">
            <span className="text-[11px] opacity-90">
              {t('systemNotification.cfpmToolGateDebugMeta', {
                action: cfpmToolGatePayload?.action ?? 'unknown',
              })}
            </span>
            {hasCommandDiff && (
              <button
                type="button"
                className="rounded border border-cyan-400/40 px-1.5 py-0.5 text-[11px] hover:bg-cyan-400/10"
                onClick={() => setShowDebugDetails((prev) => !prev)}
              >
                {showDebugDetails
                  ? t('systemNotification.cfpmHideDetails')
                  : t('systemNotification.cfpmShowDetails')}
              </button>
            )}
          </div>
        )}

        {isDebug && hasCommandDiff && showDebugDetails && (
          <div className="rounded border border-cyan-500/20 bg-black/15 px-2 py-1 text-[11px]">
            <div className="break-all">
              {t('systemNotification.cfpmToolGateOriginalCommand')}{' '}
              {cfpmToolGatePayload?.originalCommand}
            </div>
            <div className="mt-1 break-all">
              {t('systemNotification.cfpmToolGateRewrittenCommand')}{' '}
              {cfpmToolGatePayload?.rewrittenCommand}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
