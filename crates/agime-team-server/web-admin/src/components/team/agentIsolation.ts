import type { TeamAgent } from '../../api/agent';
import type { PortalSummary } from '../../api/portal';

export const DIGITAL_AVATAR_MANAGER_MARKER = '[digital-avatar-manager]';
export const DIGITAL_AVATAR_SERVICE_MARKER = '[digital-avatar-service]';

function normalizeText(value: string | undefined | null): string {
  return (value || '').trim().toLowerCase();
}

function normalizeCompact(value: string | undefined | null): string {
  return normalizeText(value).replace(/\s+/g, '');
}

export function isDigitalAvatarPortal(summary: PortalSummary): boolean {
  const tags = summary.tags || [];
  return tags.includes('digital-avatar') || tags.includes('avatar:external') || tags.includes('avatar:internal');
}

export function getDigitalAvatarManagerId(summary: PortalSummary): string | null {
  const direct = (summary.codingAgentId || summary.agentId || '').trim();
  if (direct) return direct;
  const managerTag = (summary.tags || [])
    .map(tag => tag.trim())
    .find(tag => normalizeText(tag).startsWith('manager:'));
  if (!managerTag) return null;
  const fromTag = managerTag.split(':').slice(1).join(':').trim();
  return fromTag || null;
}

export function getDigitalAvatarServiceId(summary: PortalSummary): string | null {
  const direct = (summary.serviceAgentId || summary.agentId || '').trim();
  return direct || null;
}

export function isDedicatedAvatarManager(agent: TeamAgent): boolean {
  const description = normalizeText(agent.description);
  if (description.includes(DIGITAL_AVATAR_MANAGER_MARKER)) return true;
  const compactName = normalizeCompact(agent.name);
  return compactName.includes('管理agent') || compactName.includes('manageragent');
}

export function isDedicatedAvatarService(agent: TeamAgent): boolean {
  const description = normalizeText(agent.description);
  if (description.includes(DIGITAL_AVATAR_SERVICE_MARKER)) return true;
  const compactName = normalizeCompact(agent.name);
  return compactName.endsWith('分身agent') || compactName.includes('avataragent');
}

export interface AgentIsolationResult {
  generalAgents: TeamAgent[];
  managerDedicatedAgents: TeamAgent[];
  serviceDedicatedAgents: TeamAgent[];
  dedicatedAgentIds: Set<string>;
  managerDedicatedIds: Set<string>;
  serviceDedicatedIds: Set<string>;
}

export function splitGeneralAndDedicatedAgents(
  agents: TeamAgent[],
  avatarPortals: PortalSummary[],
): AgentIsolationResult {
  const managerDedicatedIds = new Set<string>();
  const serviceDedicatedIds = new Set<string>();

  for (const portal of avatarPortals) {
    if (!isDigitalAvatarPortal(portal)) continue;
    const managerId = getDigitalAvatarManagerId(portal);
    if (managerId) managerDedicatedIds.add(managerId);
    const serviceId = getDigitalAvatarServiceId(portal);
    if (serviceId) serviceDedicatedIds.add(serviceId);
  }

  for (const agent of agents) {
    if (isDedicatedAvatarManager(agent)) managerDedicatedIds.add(agent.id);
    if (isDedicatedAvatarService(agent)) serviceDedicatedIds.add(agent.id);
  }

  const dedicatedAgentIds = new Set<string>([
    ...Array.from(managerDedicatedIds),
    ...Array.from(serviceDedicatedIds),
  ]);

  const generalAgents = agents.filter(agent => !dedicatedAgentIds.has(agent.id));
  const managerDedicatedAgents = agents.filter(agent => managerDedicatedIds.has(agent.id));
  const serviceDedicatedAgents = agents.filter(agent => serviceDedicatedIds.has(agent.id));

  return {
    generalAgents,
    managerDedicatedAgents,
    serviceDedicatedAgents,
    dedicatedAgentIds,
    managerDedicatedIds,
    serviceDedicatedIds,
  };
}
