import type { TFunction } from 'i18next';

function normalized(value: string): string {
  return `${value || ''}`.trim().toLowerCase();
}

export function getAvatarPortalStatusText(t: TFunction, status: string): string {
  const key = normalized(status);
  switch (key) {
    case 'draft':
      return t('digitalAvatar.status.draft', '草稿');
    case 'published':
      return t('digitalAvatar.status.published', '已发布');
    default:
      return status;
  }
}

export function getDigitalAvatarDocumentAccessModeText(t: TFunction, mode: string): string {
  switch (normalized(mode)) {
    case 'read_only':
      return t('digitalAvatar.documentAccessModeReadOnly', '只读');
    case 'co_edit_draft':
      return t('digitalAvatar.documentAccessModeCoEditDraft', '协作草稿');
    case 'controlled_write':
      return t('digitalAvatar.documentAccessModeControlledWrite', '受控写入');
    default:
      return mode;
  }
}

export function getDigitalAvatarRiskText(t: TFunction, risk: string): string {
  switch (normalized(risk)) {
    case 'low':
      return t('digitalAvatar.timeline.risk.low', '低风险');
    case 'medium':
      return t('digitalAvatar.timeline.risk.medium', '中风险');
    case 'high':
      return t('digitalAvatar.timeline.risk.high', '高风险');
    default:
      return risk;
  }
}

export function getDigitalAvatarStatusText(t: TFunction, value: string): string {
  const key = normalized(value);
  switch (key) {
    case 'draft':
      return t('digitalAvatar.status.draft', '草稿');
    case 'published':
      return t('digitalAvatar.status.published', '已发布');
    case 'pending':
      return t('digitalAvatar.governance.status.pending', '待决策');
    case 'approved':
      return t('digitalAvatar.governance.status.approved', '已通过');
    case 'needs_human':
      return t('digitalAvatar.governance.status.needs_human', '需人工确认');
    case 'rejected':
      return t('digitalAvatar.governance.status.rejected', '已拒绝');
    case 'pending_approval':
      return t('digitalAvatar.governance.proposalStatus.pending_approval', '待审批');
    case 'pilot':
      return t('digitalAvatar.governance.proposalStatus.pilot', '试运行');
    case 'active':
      return t('digitalAvatar.governance.proposalStatus.active', '生效中');
    case 'experimenting':
      return t('digitalAvatar.governance.ticketStatus.experimenting', '实验中');
    case 'deployed':
      return t('digitalAvatar.governance.ticketStatus.deployed', '已部署');
    case 'rolled_back':
      return t('digitalAvatar.governance.ticketStatus.rolled_back', '已回滚');
    case 'failed':
      return t('digitalAvatar.audit.eventStatus.failed', '失败');
    case 'created':
      return t('digitalAvatar.audit.eventStatus.created', '已创建');
    case 'updated':
      return t('digitalAvatar.audit.eventStatus.updated', '已更新');
    case 'success':
      return t('digitalAvatar.audit.eventStatus.success', '成功');
    case 'error':
      return t('digitalAvatar.governance.runtimeSeverity.error', '错误');
    case 'warn':
      return t('digitalAvatar.governance.runtimeSeverity.warn', '警告');
    case 'info':
      return t('digitalAvatar.governance.runtimeSeverity.info', '正常');
    default:
      return value;
  }
}

export function formatDigitalAvatarMetaLabel(t: TFunction, value: string): string {
  const text = `${value || ''}`.trim();
  if (!text) return text;
  if (text.startsWith('执行人:')) return text;
  if (text.startsWith('事件:')) {
    const eventType = text.slice(3).trim();
    return `${t('digitalAvatar.audit.eventType', { defaultValue: '事件' })}:${getDigitalAvatarStatusText(t, eventType)}`;
  }
  if (text.includes('high') || text.includes('medium') || text.includes('low')) {
    const lower = text.toLowerCase();
    if (lower === 'high' || lower === 'medium' || lower === 'low') {
      return getDigitalAvatarRiskText(t, lower);
    }
  }
  return getDigitalAvatarStatusText(t, text);
}

