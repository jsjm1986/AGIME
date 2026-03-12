import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/select';
import { TeamAgent } from '../../api/agent';
import { AgentAvatar } from '../agent/AvatarPicker';
import { fetchVisibleChatAgents } from './visibleChatAgents';

const agentStatusDot: Record<string, string> = {
  running: 'bg-status-info-text',
  error: 'bg-status-error-text',
  paused: 'bg-status-warning-text',
};

interface AgentSelectorProps {
  teamId: string;
  selectedAgentId: string | null;
  onSelect: (agent: TeamAgent) => void;
  onClear?: () => void;
  compact?: boolean;
}

export function AgentSelector({
  teamId,
  selectedAgentId,
  onSelect,
  onClear,
  compact,
}: AgentSelectorProps) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    let cancelled = false;
    const load = async () => {
      try {
        const visibleAgents = await fetchVisibleChatAgents(teamId);
        if (!cancelled) setAgents(visibleAgents);
      } catch (e) {
        console.error('Failed to load agents:', e);
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    load();
    return () => { cancelled = true; };
  }, [teamId]);

  const handleChange = (value: string) => {
    if (value === '__all__') {
      onClear?.();
      return;
    }
    const agent = agents.find(a => a.id === value);
    if (agent) onSelect(agent);
  };

  const triggerClass = compact
    ? 'w-full h-8 text-xs'
    : 'w-full h-9 text-[13px]';
  const effectiveValue = agents.some(agent => agent.id === selectedAgentId)
    ? selectedAgentId || '__all__'
    : '__all__';

  if (loading) {
    return (
      <Select disabled>
        <SelectTrigger className={triggerClass}>
          <SelectValue placeholder={t('common.loading', 'Loading...')} />
        </SelectTrigger>
      </Select>
    );
  }

  return (
    <Select value={effectiveValue} onValueChange={handleChange}>
      <SelectTrigger className={triggerClass}>
        <SelectValue placeholder={t('chat.selectAgent', 'Select an agent')} />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="__all__">
          <span className="text-muted-foreground">{t('chat.allAgents', 'All Agents')}</span>
        </SelectItem>
        {agents.map(agent => (
          <SelectItem key={agent.id} value={agent.id}>
            <div className="flex items-center gap-2">
              <AgentAvatar avatar={agent.avatar} name={agent.name} className="h-4 w-4 bg-transparent" iconSize="h-3.5 w-3.5" />
              <span>{agent.name}</span>
              <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${
                agentStatusDot[agent.status] || 'bg-status-success-text'
              }`} />
            </div>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
