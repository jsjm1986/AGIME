import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot } from 'lucide-react';
import { Select, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../ui/select';
import { agentApi, TeamAgent } from '../../api/agent';

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
        const res = await agentApi.listAgents(teamId);
        if (!cancelled) setAgents(res.items || []);
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
    : 'w-full h-9 text-sm';

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
    <Select value={selectedAgentId || '__all__'} onValueChange={handleChange}>
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
              <Bot className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
              <span>{agent.name}</span>
              <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${
                agent.status === 'running' ? 'bg-green-500' :
                agent.status === 'error' ? 'bg-red-500' :
                agent.status === 'paused' ? 'bg-amber-500' : 'bg-slate-400'
              }`} />
            </div>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
