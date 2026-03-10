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
        className: 'border-violet-200 bg-violet-50 text-violet-700 dark:border-violet-900/60 dark:bg-violet-950/30 dark:text-violet-300',
        icon: ShieldCheck,
      };
    case 'avatar_service':
      return {
        label: t('agent.type.avatarService', { defaultValue: '分身服务 Agent' }),
        className: 'border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/60 dark:bg-emerald-950/30 dark:text-emerald-300',
        icon: UserRound,
      };
    case 'ecosystem':
      return {
        label: t('agent.type.ecosystem', { defaultValue: '生态协作 Agent' }),
        className: 'border-sky-200 bg-sky-50 text-sky-700 dark:border-sky-900/60 dark:bg-sky-950/30 dark:text-sky-300',
        icon: Globe2,
      };
    case 'general':
    default:
      return {
        label: t('agent.type.general', { defaultValue: '常规 Agent' }),
        className: 'border-slate-200 bg-slate-50 text-slate-700 dark:border-slate-800 dark:bg-slate-900/40 dark:text-slate-300',
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
