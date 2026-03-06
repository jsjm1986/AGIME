import { agentApi, type TeamAgent } from '../../api/agent';
import { portalApi } from '../../api/portal';
import { splitGeneralAndDedicatedAgents } from '../team/agentIsolation';

export async function fetchVisibleChatAgents(teamId: string): Promise<TeamAgent[]> {
  const [agentResult, avatarResult] = await Promise.all([
    agentApi.listAgents(teamId, 1, 200),
    portalApi.list(teamId, 1, 200, 'avatar'),
  ]);

  const allAgents = agentResult.items || [];
  const avatars = avatarResult.items || [];
  return splitGeneralAndDedicatedAgents(allAgents, avatars).generalAgents;
}

export function isVisibleChatAgent(agents: TeamAgent[], agentId: string | null | undefined): boolean {
  if (!agentId) return false;
  return agents.some(agent => agent.id === agentId);
}
