import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from '../../ui/dialog';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import {
  portalApi,
  type CreatePortalRequest,
  type PortalDocumentAccessMode,
} from '../../../api/portal';
import { agentApi, type TeamAgent } from '../../../api/agent';
import {
  classifyPortalServiceAgent,
  describePortalServiceBindingMode,
  formatPortalServiceAgentOptionLabel,
  groupPortalServiceAgents,
} from './serviceAgentBinding';

interface CreatePortalDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  portalBaseUrl?: string | null;
  onCreated: () => void;
}

function slugify(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, '')
    .trim()
    .replace(/[\s]+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '');
}

export function CreatePortalDialog({ open, onOpenChange, teamId, portalBaseUrl, onCreated }: CreatePortalDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [slugManual, setSlugManual] = useState(false);
  const [description, setDescription] = useState('');
  const [codingAgentId, setCodingAgentId] = useState('');
  const [serviceAgentId, setServiceAgentId] = useState('');
  const [documentAccessMode, setDocumentAccessMode] =
    useState<PortalDocumentAccessMode>('read_only');
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const findAgentById = (id: string) => agents.find(agent => agent.id === id) || null;
  const effectiveServiceSourceId = serviceAgentId || codingAgentId;
  const effectiveServiceSourceAgent = effectiveServiceSourceId ? findAgentById(effectiveServiceSourceId) : null;
  const serviceBindingMode = classifyPortalServiceAgent(effectiveServiceSourceAgent);

  const serviceAgentGroups = groupPortalServiceAgents(agents);

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
      let effectiveServiceAgentId: string | undefined;
      if (effectiveServiceSourceId) {
        const sourceAgent = effectiveServiceSourceAgent;
        if (!sourceAgent) {
          setError(t('laboratory.serviceAgentSourceMissing', '未找到选中的服务 Agent。'));
          return;
        }
        switch (serviceBindingMode) {
          case 'clone_general': {
            const dedicated = await agentApi.provisionFromTemplate(sourceAgent.id, {
              name: `${name.trim()} ${t('laboratory.serviceAgentNameSuffix', '服务Agent')}`.trim(),
              agent_domain: 'ecosystem_portal',
              agent_role: 'service',
              template_source_agent_id: sourceAgent.id,
            });
            effectiveServiceAgentId = dedicated.id;
            break;
          }
          case 'direct_ecosystem':
          case 'shared_avatar':
            effectiveServiceAgentId = sourceAgent.id;
            break;
          case 'invalid_avatar_manager':
          case 'invalid_other':
            setError(describePortalServiceBindingMode(t, serviceBindingMode));
            return;
          case 'none':
            break;
        }
      }

      const req: CreatePortalRequest = {
        name: name.trim(),
        description: description.trim() || undefined,
        agentEnabled: !!(effectiveServiceAgentId || codingAgentId),
        codingAgentId: codingAgentId || undefined,
        serviceAgentId: effectiveServiceAgentId,
        documentAccessMode,
      };
      if (slug.trim()) req.slug = slug.trim();
      await portalApi.create(teamId, req);
      onCreated();
      onOpenChange(false);
      setName('');
      setSlug('');
      setSlugManual(false);
      setDescription('');
      setCodingAgentId('');
      setServiceAgentId('');
      setDocumentAccessMode('read_only');
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
              onChange={(e) => {
                setName(e.target.value);
                if (!slugManual) setSlug(slugify(e.target.value));
              }}
              placeholder="My Portal"
              autoFocus
            />
          </div>
          <div>
            <label className="text-sm font-medium">{t('laboratory.slug')}</label>
            <Input
              value={slug}
              onChange={(e) => {
                setSlugManual(true);
                setSlug(e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '-'));
              }}
              placeholder={t('laboratory.slugHint')}
            />
            {slug && (
              <p className="text-xs text-muted-foreground mt-1.5 font-mono break-all">
                {(portalBaseUrl || '') + '/p/' + slug}
              </p>
            )}
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
              {t('laboratory.codingAgentHint', 'Used for Portal ecosystem collaboration coding sessions')}
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
              {serviceAgentGroups.general.length > 0 && (
                <optgroup label={t('laboratory.serviceAgentGroupGeneral', '通用模板（将复制）')}>
                  {serviceAgentGroups.general.map(a => (
                    <option key={a.id} value={a.id}>{formatPortalServiceAgentOptionLabel(t, a)}</option>
                  ))}
                </optgroup>
              )}
              {serviceAgentGroups.ecosystem.length > 0 && (
                <optgroup label={t('laboratory.serviceAgentGroupEcosystem', '生态专用服务（直接接入）')}>
                  {serviceAgentGroups.ecosystem.map(a => (
                    <option key={a.id} value={a.id}>{formatPortalServiceAgentOptionLabel(t, a)}</option>
                  ))}
                </optgroup>
              )}
              {serviceAgentGroups.avatar.length > 0 && (
                <optgroup label={t('laboratory.serviceAgentGroupAvatar', '数字分身服务（共享接入）')}>
                  {serviceAgentGroups.avatar.map(a => (
                    <option key={a.id} value={a.id}>{formatPortalServiceAgentOptionLabel(t, a)}</option>
                  ))}
                </optgroup>
              )}
              {serviceAgentGroups.blocked.length > 0 && (
                <optgroup label={t('laboratory.serviceAgentGroupBlocked', '不可用于服务')}>
                  {serviceAgentGroups.blocked.map(a => (
                    <option key={a.id} value={a.id} disabled>{formatPortalServiceAgentOptionLabel(t, a)}</option>
                  ))}
                </optgroup>
              )}
            </select>
            <p className="text-xs text-muted-foreground mt-1">
              {describePortalServiceBindingMode(t, serviceBindingMode)}
            </p>
          </div>
          <div>
            <label className="text-sm font-medium">
              {t('laboratory.documentAccessMode', 'Document Access Mode')}
            </label>
            <select
              className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm"
              value={documentAccessMode}
              onChange={(e) => setDocumentAccessMode(e.target.value as PortalDocumentAccessMode)}
            >
              <option value="read_only">
                {t('laboratory.documentAccessModeReadOnly', 'Read only')}
              </option>
              <option value="co_edit_draft">
                {t('laboratory.documentAccessModeCoEditDraft', 'Collaborative draft')}
              </option>
              <option value="controlled_write">
                {t('laboratory.documentAccessModeControlledWrite', 'Controlled write')}
              </option>
            </select>
            <p className="text-xs text-muted-foreground mt-1">
              {documentAccessMode === 'read_only' &&
                t(
                  'laboratory.documentAccessModeReadOnlyHint',
                  'Visitors can only read/search/list bound documents.'
                )}
              {documentAccessMode === 'co_edit_draft' &&
                t(
                  'laboratory.documentAccessModeCoEditDraftHint',
                  'Visitors can create/update agent drafts within bound scope.'
                )}
              {documentAccessMode === 'controlled_write' &&
                t(
                  'laboratory.documentAccessModeControlledWriteHint',
                  'Visitors can write with stricter policy controls.'
                )}
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
