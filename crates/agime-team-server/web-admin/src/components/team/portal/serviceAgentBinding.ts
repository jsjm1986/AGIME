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
