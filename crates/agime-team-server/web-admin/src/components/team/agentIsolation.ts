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
  if (agent.agent_domain === 'digital_avatar' && agent.agent_role === 'manager') {
    return true;
  }
  const description = normalizeText(agent.description);
  if (description.includes(DIGITAL_AVATAR_MANAGER_MARKER)) return true;
  const compactName = normalizeCompact(agent.name);
  return compactName.includes('管理agent') || compactName.includes('manageragent');
}

export function isDedicatedAvatarService(agent: TeamAgent): boolean {
  if (agent.agent_domain === 'digital_avatar' && agent.agent_role === 'service') {
    return true;
  }
  const description = normalizeText(agent.description);
  if (description.includes(DIGITAL_AVATAR_SERVICE_MARKER)) return true;
  const compactName = normalizeCompact(agent.name);
  return compactName.endsWith('分身agent') || compactName.includes('avataragent');
}

export function isDedicatedEcosystemService(agent: TeamAgent): boolean {
  const domain = normalizeText(agent.agent_domain);
  const role = normalizeText(agent.agent_role);
  return domain === 'ecosystem_portal' && (!role || role === 'service');
}

export interface AgentIsolationResult {
  generalAgents: TeamAgent[];
  managerDedicatedAgents: TeamAgent[];
  serviceDedicatedAgents: TeamAgent[];
  ecosystemDedicatedAgents: TeamAgent[];
  dedicatedAgentIds: Set<string>;
  managerDedicatedIds: Set<string>;
  serviceDedicatedIds: Set<string>;
  ecosystemDedicatedIds: Set<string>;
}

export interface DedicatedAvatarPortalLink {
  portalId: string;
  portalName: string;
  portalSlug: string;
  portalStatus: string;
  serviceAgent: TeamAgent | null;
}

export interface DedicatedAvatarGroup {
  managerId: string;
  managerAgent: TeamAgent | null;
  managerRoles: Array<'manager' | 'service'>;
  portals: DedicatedAvatarPortalLink[];
}

export interface DedicatedAvatarGroupingResult {
  generalAgents: TeamAgent[];
  dedicatedGroups: DedicatedAvatarGroup[];
  hiddenDedicatedCount: number;
}

export const UNGROUPED_MANAGER_KEY = '__ungrouped__';

export function splitGeneralAndDedicatedAgents(
  agents: TeamAgent[],
  _avatarPortals: PortalSummary[],
): AgentIsolationResult {
  const managerDedicatedIds = new Set<string>();
  const serviceDedicatedIds = new Set<string>();
  const ecosystemDedicatedIds = new Set<string>();

  for (const agent of agents) {
    if (isDedicatedAvatarManager(agent)) managerDedicatedIds.add(agent.id);
    if (isDedicatedAvatarService(agent)) serviceDedicatedIds.add(agent.id);
    if (isDedicatedEcosystemService(agent)) ecosystemDedicatedIds.add(agent.id);
  }

  const dedicatedAgentIds = new Set<string>([
    ...Array.from(managerDedicatedIds),
    ...Array.from(serviceDedicatedIds),
    ...Array.from(ecosystemDedicatedIds),
  ]);

  const generalAgents = agents.filter(agent => !dedicatedAgentIds.has(agent.id));
  const managerDedicatedAgents = agents.filter(agent => managerDedicatedIds.has(agent.id));
  const serviceDedicatedAgents = agents.filter(agent => serviceDedicatedIds.has(agent.id));
  const ecosystemDedicatedAgents = agents.filter(agent => ecosystemDedicatedIds.has(agent.id));

  return {
    generalAgents,
    managerDedicatedAgents,
    serviceDedicatedAgents,
    ecosystemDedicatedAgents,
    dedicatedAgentIds,
    managerDedicatedIds,
    serviceDedicatedIds,
    ecosystemDedicatedIds,
  };
}

export function buildDedicatedAvatarGrouping(
  agents: TeamAgent[],
  avatarPortals: PortalSummary[],
): DedicatedAvatarGroupingResult {
  const {
    generalAgents,
    dedicatedAgentIds,
    managerDedicatedIds,
    serviceDedicatedIds,
  } = splitGeneralAndDedicatedAgents(agents, avatarPortals);
  const agentById = new Map(agents.map(agent => [agent.id, agent]));
  const usedServiceAgentIds = new Set<string>();
  const groupMap = new Map<string, DedicatedAvatarGroup>();

  const ensureGroup = (managerId: string): DedicatedAvatarGroup => {
    const existing = groupMap.get(managerId);
    if (existing) return existing;
    const managerAgent = managerId === UNGROUPED_MANAGER_KEY ? null : agentById.get(managerId) || null;
    const managerRoles: Array<'manager' | 'service'> = [];
    if (managerAgent) {
      if (managerDedicatedIds.has(managerAgent.id)) managerRoles.push('manager');
      if (serviceDedicatedIds.has(managerAgent.id)) managerRoles.push('service');
    }
    const nextGroup: DedicatedAvatarGroup = {
      managerId,
      managerAgent,
      managerRoles,
      portals: [],
    };
    groupMap.set(managerId, nextGroup);
    return nextGroup;
  };

  for (const portal of avatarPortals) {
    if (!isDigitalAvatarPortal(portal)) continue;
    const managerId = getDigitalAvatarManagerId(portal);
    const groupManagerId =
      managerId && managerDedicatedIds.has(managerId) ? managerId : UNGROUPED_MANAGER_KEY;
    const serviceId = getDigitalAvatarServiceId(portal);
    const serviceAgent = serviceId ? agentById.get(serviceId) || null : null;
    if (serviceAgent) usedServiceAgentIds.add(serviceAgent.id);
    ensureGroup(groupManagerId).portals.push({
      portalId: portal.id,
      portalName: portal.name,
      portalSlug: portal.slug,
      portalStatus: portal.status,
      serviceAgent,
    });
  }

  for (const agent of agents) {
    if (managerDedicatedIds.has(agent.id) && !groupMap.has(agent.id)) {
      ensureGroup(agent.id);
    }
  }

  for (const agent of agents) {
    if (!serviceDedicatedIds.has(agent.id) || usedServiceAgentIds.has(agent.id)) continue;
    const fallbackManagerId = agent.owner_manager_agent_id?.trim() || UNGROUPED_MANAGER_KEY;
    ensureGroup(fallbackManagerId).portals.push({
      portalId: `orphan:${agent.id}`,
      portalName: agent.name,
      portalSlug: '',
      portalStatus: 'draft',
      serviceAgent: agent,
    });
  }

  const dedicatedGroups = Array.from(groupMap.values()).sort((a, b) => {
    const aName = a.managerAgent?.name || '';
    const bName = b.managerAgent?.name || '';
    if (a.managerId === UNGROUPED_MANAGER_KEY) return 1;
    if (b.managerId === UNGROUPED_MANAGER_KEY) return -1;
    return aName.localeCompare(bName, 'zh-CN');
  });

  return {
    generalAgents,
    dedicatedGroups,
    hiddenDedicatedCount: dedicatedAgentIds.size,
  };
}
