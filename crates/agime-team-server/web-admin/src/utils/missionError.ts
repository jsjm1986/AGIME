import type { TFunction } from 'i18next';

function stripTaskPrefix(message: string): string {
  // Example: "Task xxx failed: API key not configured for agent 'A'"
  return message.replace(/^Task\s+[^\s]+\s+failed:\s*/i, '').trim();
}

export function localizeMissionError(
  message: string | null | undefined,
  t: TFunction,
): string {
  const raw = (message || '').trim();
  if (!raw) return '';

  const normalized = stripTaskPrefix(raw);

  if (/No pending steps available to resume/i.test(normalized)) {
    return t(
      'mission.errorNoPendingSteps',
      '当前没有可继续执行的待处理步骤',
    );
  }

  if (/Mission must be in Draft or Planned status to start/i.test(normalized)) {
    return t(
      'mission.errorInvalidStartState',
      '当前任务状态不允许启动，请刷新后重试',
    );
  }
  if (
    /mission status transition rejected/i.test(normalized) ||
    /mission not found or transition rejected/i.test(normalized)
  ) {
    return t(
      'mission.errorInvalidStartState',
      '当前任务状态不允许启动，请刷新后重试',
    );
  }

  const apiKeyMatch = normalized.match(
    /API key not configured for agent ['"]([^'"]+)['"]/i,
  );
  if (apiKeyMatch) {
    return t('mission.errorAgentApiKeyMissing', {
      agent: apiKeyMatch[1],
      defaultValue: 'Agent {{agent}} 未配置 API Key',
    });
  }

  if (
    /Stream decode error|error decoding response body|tls handshake eof|handshake eof/i.test(
      normalized,
    )
  ) {
    return t(
      'mission.errorStreamUnstable',
      '模型流式响应不稳定，系统已自动重试，请稍后继续',
    );
  }

  return raw;
}
