import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from '../../ui/dialog';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { portalApi, type CreatePortalRequest } from '../../../api/portal';
import { agentApi, type TeamAgent } from '../../../api/agent';

interface CreatePortalDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  onCreated: () => void;
}

export function CreatePortalDialog({ open, onOpenChange, teamId, onCreated }: CreatePortalDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [description, setDescription] = useState('');
  const [codingAgentId, setCodingAgentId] = useState('');
  const [serviceAgentId, setServiceAgentId] = useState('');
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  // Load agents list when dialog opens
  useEffect(() => {
    if (open) {
      agentApi.listAgents(teamId).then(res => setAgents(res.items || [])).catch(() => {});
    }
  }, [open, teamId]);

  const handleCreate = async () => {
    if (!name.trim()) return;
    setLoading(true);
    setError('');
    try {
      const req: CreatePortalRequest = {
        name: name.trim(),
        description: description.trim() || undefined,
        agentEnabled: !!(serviceAgentId || codingAgentId),
        codingAgentId: codingAgentId || undefined,
        serviceAgentId: (serviceAgentId || codingAgentId) || undefined,
      };
      if (slug.trim()) req.slug = slug.trim();
      await portalApi.create(teamId, req);
      onCreated();
      onOpenChange(false);
      setName('');
      setSlug('');
      setDescription('');
      setCodingAgentId('');
      setServiceAgentId('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t('laboratory.createPortal')}</DialogTitle>
        </DialogHeader>
        <div className="space-y-4 py-2">
          <div>
            <label className="text-sm font-medium">{t('laboratory.portalName')}</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="My Portal"
              autoFocus
            />
          </div>
          <div>
            <label className="text-sm font-medium">{t('laboratory.slug')}</label>
            <Input
              value={slug}
              onChange={(e) => setSlug(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '-'))}
              placeholder={t('laboratory.slugHint')}
            />
          </div>
          <div>
            <label className="text-sm font-medium">{t('laboratory.portalDescription')}</label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>
          <div>
            <label className="text-sm font-medium">{t('laboratory.codingAgentSelect', 'Coding Agent')}</label>
            <select
              className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm"
              value={codingAgentId}
              onChange={(e) => setCodingAgentId(e.target.value)}
            >
              <option value="">{t('laboratory.noAgentSelected')}</option>
              {agents.map(a => (
                <option key={a.id} value={a.id}>{a.name}{a.model ? ` (${a.model})` : ''}</option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground mt-1">
              {t('laboratory.codingAgentHint', 'Used for Portal laboratory coding sessions')}
            </p>
          </div>
          <div>
            <label className="text-sm font-medium">{t('laboratory.serviceAgentSelect', 'Service Agent')}</label>
            <select
              className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm"
              value={serviceAgentId}
              onChange={(e) => setServiceAgentId(e.target.value)}
            >
              <option value="">{t('laboratory.followCodingAgent', 'Follow coding agent')}</option>
              {agents.map(a => (
                <option key={a.id} value={a.id}>{a.name}{a.model ? ` (${a.model})` : ''}</option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground mt-1">
              {t('laboratory.serviceAgentHint', 'Used for public visitor chat on /p/{slug}')}
            </p>
          </div>
          {error && <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>{t('common.cancel')}</Button>
          <Button onClick={handleCreate} disabled={loading || !name.trim()}>
            {loading ? t('common.creating') : t('common.create')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
