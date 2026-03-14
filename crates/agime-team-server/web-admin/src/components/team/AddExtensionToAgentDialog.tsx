import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ApiError } from '../../api/client';
import { Button } from '../ui/button';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { agentApi, type TeamAgent } from '../../api/agent';

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  extensionId: string;
  extensionName: string;
  teamId: string;
}

export function AddExtensionToAgentDialog({
  open,
  onOpenChange,
  extensionId,
  extensionName,
  teamId,
}: Props) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedAgentId, setSelectedAgentId] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');
  const [success, setSuccess] = useState('');
  const [alreadyAddedAgentIds, setAlreadyAddedAgentIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!open) {
      setSelectedAgentId('');
      setError('');
      setSuccess('');
      setAlreadyAddedAgentIds(new Set());
      return;
    }
    loadAgents();
  }, [open, teamId]);

  const loadAgents = async () => {
    setLoading(true);
    try {
      const res = await agentApi.listAgents(teamId, 1, 100);
      setAgents(res.items);
      const next = new Set<string>();
      for (const agent of res.items) {
        const exists = (agent.custom_extensions || []).some((ext) => {
          const sourceId = ext.source_extension_id || '';
          return sourceId === extensionId || ext.name === extensionName;
        });
        if (exists) next.add(agent.id);
      }
      setAlreadyAddedAgentIds(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const handleSubmit = async () => {
    if (!selectedAgentId) return;
    if (alreadyAddedAgentIds.has(selectedAgentId)) {
      setSuccess(t('teams.resource.alreadyAddedToAgent'));
      return;
    }
    setSubmitting(true);
    setError('');
    setSuccess('');
    try {
      await agentApi.addTeamExtension(selectedAgentId, extensionId, teamId);
      const agentName = agents.find(a => a.id === selectedAgentId)?.name || '';
      setAlreadyAddedAgentIds((prev) => {
        const next = new Set(prev);
        next.add(selectedAgentId);
        return next;
      });
      setSuccess(
        t('teams.resource.addToAgentSuccess', {
          extension: extensionName,
          agent: agentName,
        })
      );
      setTimeout(() => onOpenChange(false), 1500);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        setSuccess(t('teams.resource.alreadyAddedToAgent'));
        setError('');
        return;
      }
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[420px]">
        <DialogHeader>
          <DialogTitle>
            {t('teams.resource.addToAgent')}
          </DialogTitle>
        </DialogHeader>

        <div className="py-4 space-y-3">
          <p className="text-sm text-[hsl(var(--muted-foreground))]">
            {t('teams.resource.addToAgentDesc', { name: extensionName })}
          </p>
          <p className="text-xs text-[hsl(var(--muted-foreground))]">
            {t('teams.resource.addToAgentTip')}
          </p>

          {loading ? (
            <p className="text-sm text-center py-4">
              {t('common.loading')}
            </p>
          ) : agents.length === 0 ? (
            <p className="text-sm text-center py-4 text-[hsl(var(--muted-foreground))]">
              {t('teams.resource.noAgents')}
            </p>
          ) : (
            <select
              className="w-full px-3 py-2 rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] text-sm"
              value={selectedAgentId}
              onChange={(e) => setSelectedAgentId(e.target.value)}
            >
              <option value="">
                {t('teams.resource.selectAgent')}
              </option>
              {agents.map((agent) => (
                <option key={agent.id} value={agent.id} disabled={alreadyAddedAgentIds.has(agent.id)}>
                  {agent.name}
                  {agent.status !== 'idle' ? ` (${agent.status})` : ''}
                  {alreadyAddedAgentIds.has(agent.id) ? ` - ${t('teams.resource.alreadyAddedTag')}` : ''}
                </option>
              ))}
            </select>
          )}
          {agents.length > 0 && alreadyAddedAgentIds.size === agents.length && (
            <p className="text-sm text-[hsl(var(--muted-foreground))]">
              {t('teams.resource.allAgentsAlreadyAdded')}
            </p>
          )}

          {error && (
            <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>
          )}
          {success && (
            <p className="text-sm text-status-success-text">{success}</p>
          )}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            {t('common.cancel')}
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={!selectedAgentId || submitting || alreadyAddedAgentIds.size === agents.length}
          >
            {submitting ? t('common.loading') : t('common.add')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
