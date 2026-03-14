import { Bot, Globe2, ShieldCheck, UserRound } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { TeamAgent } from '../../api/agent';
import { Badge } from '../ui/badge';
import { isDedicatedAvatarManager, isDedicatedAvatarService } from '../team/agentIsolation';

export type AgentVisualType = 'general' | 'avatar_manager' | 'avatar_service' | 'ecosystem';

export function resolveAgentVisualType(agent: TeamAgent): AgentVisualType {
  if (agent.agent_domain === 'digital_avatar' && agent.agent_role === 'manager') {
    return 'avatar_manager';
  }
  if (agent.agent_domain === 'digital_avatar' && agent.agent_role === 'service') {
    return 'avatar_service';
  }
  if (agent.agent_domain === 'ecosystem_portal') {
    return 'ecosystem';
  }
  if (isDedicatedAvatarManager(agent)) {
    return 'avatar_manager';
  }
  if (isDedicatedAvatarService(agent)) {
    return 'avatar_service';
  }
  return 'general';
}

function getAgentTypeMeta(
  type: AgentVisualType,
  t: ReturnType<typeof useTranslation>['t'],
): { label: string; className: string; icon: typeof Bot } {
  switch (type) {
    case 'avatar_manager':
      return {
        label: t('agent.type.avatarManager', { defaultValue: '分身管理 Agent' }),
        className: 'border-[hsl(var(--status-info-text))/0.16] bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]',
        icon: ShieldCheck,
      };
    case 'avatar_service':
      return {
        label: t('agent.type.avatarService', { defaultValue: '分身服务 Agent' }),
        className: 'border-[hsl(var(--status-success-text))/0.16] bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]',
        icon: UserRound,
      };
    case 'ecosystem':
      return {
        label: t('agent.type.ecosystem', { defaultValue: '生态协作 Agent' }),
        className: 'border-[hsl(var(--status-info-text))/0.16] bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]',
        icon: Globe2,
      };
    case 'general':
    default:
      return {
        label: t('agent.type.general', { defaultValue: '常规 Agent' }),
        className: 'border-[hsl(var(--status-neutral-text))/0.14] bg-[hsl(var(--status-neutral-bg))] text-[hsl(var(--status-neutral-text))]',
        icon: Bot,
      };
  }
}

export function AgentTypeBadge({
  type,
  className = '',
}: {
  type: AgentVisualType;
  className?: string;
}) {
  const { t } = useTranslation();
  const meta = getAgentTypeMeta(type, t);
  const Icon = meta.icon;

  return (
    <Badge
      variant="outline"
      className={`inline-flex items-center gap-1 border text-[11px] ${meta.className} ${className}`.trim()}
    >
      <Icon className="h-3 w-3" />
      <span>{meta.label}</span>
    </Badge>
  );
}
