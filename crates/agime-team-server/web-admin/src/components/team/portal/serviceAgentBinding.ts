import type { TFunction } from 'i18next';
import type { TeamAgent } from '../../../api/agent';

export type PortalServiceBindingMode =
  | 'none'
  | 'clone_general'
  | 'direct_ecosystem'
  | 'shared_avatar'
  | 'invalid_avatar_manager'
  | 'invalid_other';

function normalize(value?: string | null): string {
  return (value || '').trim().toLowerCase();
}

export function classifyPortalServiceAgent(agent?: TeamAgent | null): PortalServiceBindingMode {
  if (!agent) return 'none';

  const domain = normalize(agent.agent_domain);
  const role = normalize(agent.agent_role);

  if (!domain || domain === 'general') {
    return 'clone_general';
  }

  if (domain === 'digital_avatar') {
    if (role === 'service') return 'shared_avatar';
    if (role === 'manager') return 'invalid_avatar_manager';
    return 'invalid_other';
  }

  if (domain === 'ecosystem_portal') {
    if (!role || role === 'service') return 'direct_ecosystem';
    return 'invalid_other';
  }

  return 'invalid_other';
}

export function isPortalServiceAgentSelectable(agent?: TeamAgent | null): boolean {
  const mode = classifyPortalServiceAgent(agent);
  return mode === 'clone_general' || mode === 'direct_ecosystem' || mode === 'shared_avatar';
}

export function isSharedAvatarServiceAgent(agent?: TeamAgent | null): boolean {
  return classifyPortalServiceAgent(agent) === 'shared_avatar';
}

export function describePortalServiceBindingMode(
  t: TFunction,
  mode: PortalServiceBindingMode,
): string {
  switch (mode) {
    case 'clone_general':
      return t(
        'laboratory.serviceBindingCloneTemplate',
        '将复制为生态协作专用服务 Agent，原通用 Agent 不会被修改。',
      );
    case 'direct_ecosystem':
      return t(
        'laboratory.serviceBindingDirectEcosystem',
        '这是生态协作专用服务 Agent，可直接接入并在生态协作中治理。',
      );
    case 'shared_avatar':
      return t(
        'laboratory.serviceBindingSharedAvatar',
        '这是数字分身服务 Agent，将以共享服务方式接入，底层能力治理仍归数字分身频道。',
      );
    case 'invalid_avatar_manager':
      return t(
        'laboratory.serviceBindingInvalidAvatarManager',
        '数字分身管理 Agent 只能治理分身，不能直接作为生态协作的服务 Agent。',
      );
    case 'invalid_other':
      return t(
        'laboratory.serviceBindingInvalidOther',
        '该 Agent 类型不适合作为生态协作的服务 Agent，请改选通用模板、生态专用服务或数字分身服务。',
      );
    default:
      return t(
        'laboratory.serviceBindingHintDefault',
        '服务 Agent 负责对外访客会话。选择通用 Agent 时，系统会先复制成生态协作专用服务 Agent。',
      );
  }
}

export function formatPortalServiceAgentOptionLabel(
  t: TFunction,
  agent: TeamAgent,
): string {
  const suffix = agent.model ? ` (${agent.model})` : '';
  switch (classifyPortalServiceAgent(agent)) {
    case 'clone_general':
      return `[${t('laboratory.serviceAgentTypeGeneralTemplate', '通用模板')}] ${agent.name}${suffix}`;
    case 'direct_ecosystem':
      return `[${t('laboratory.serviceAgentTypeEcosystemService', '生态专用服务')}] ${agent.name}${suffix}`;
    case 'shared_avatar':
      return `[${t('laboratory.serviceAgentTypeAvatarService', '分身服务')}] ${agent.name}${suffix}`;
    case 'invalid_avatar_manager':
      return `[${t('laboratory.serviceAgentTypeAvatarManagerBlocked', '不可用于服务')}] ${agent.name}${suffix}`;
    default:
      return `[${t('laboratory.serviceAgentTypeUnsupported', '不建议使用')}] ${agent.name}${suffix}`;
  }
}

export function groupPortalServiceAgents(agents: TeamAgent[]): {
  general: TeamAgent[];
  ecosystem: TeamAgent[];
  avatar: TeamAgent[];
  blocked: TeamAgent[];
} {
  return {
    general: agents.filter((agent) => classifyPortalServiceAgent(agent) === 'clone_general'),
    ecosystem: agents.filter((agent) => classifyPortalServiceAgent(agent) === 'direct_ecosystem'),
    avatar: agents.filter((agent) => classifyPortalServiceAgent(agent) === 'shared_avatar'),
    blocked: agents.filter((agent) => {
      const mode = classifyPortalServiceAgent(agent);
      return mode === 'invalid_avatar_manager' || mode === 'invalid_other';
    }),
  };
}

export function getPortalServiceBindingBadgeMeta(
  t: TFunction,
  mode: PortalServiceBindingMode,
): { label: string; className: string } | null {
  if (mode === 'shared_avatar') {
    return {
      label: t('laboratory.serviceBindingModeBadgeShared', '共享分身服务'),
      className: 'border-[hsl(var(--status-warning-text))/0.18] bg-[hsl(var(--status-warning-bg))] text-[hsl(var(--status-warning-text))]',
    };
  }
  if (mode === 'direct_ecosystem') {
    return {
      label: t('laboratory.serviceBindingModeBadgeDedicated', '生态专用服务'),
      className: 'border-[hsl(var(--status-success-text))/0.18] bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]',
    };
  }
  if (mode === 'clone_general') {
    return {
      label: t('laboratory.serviceBindingModeBadgeCloneOnSave', '保存后复制为专用服务'),
      className: 'border-[hsl(var(--status-info-text))/0.18] bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]',
    };
  }
  return null;
}
