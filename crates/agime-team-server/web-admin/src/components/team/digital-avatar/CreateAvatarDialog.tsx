import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../ui/dialog';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { portalApi, type PortalDetail, type PortalDocumentAccessMode } from '../../../api/portal';
import { agentApi, type TeamAgent } from '../../../api/agent';
import { DIGITAL_AVATAR_SERVICE_MARKER, splitGeneralAndDedicatedAgents } from '../agentIsolation';

type AvatarType = 'external_service' | 'internal_worker';
type AvatarRunMode = 'on_demand' | 'scheduled' | 'event_driven';

interface CreateAvatarDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  managerAgentId: string | null;
  managerAgentName?: string | null;
  onCreated: (avatar: PortalDetail) => void;
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

function buildDedicatedServiceAgentName(avatarName: string): string {
  const suffix = '分身Agent';
  const raw = `${avatarName} - ${suffix}`.trim();
  if (raw.length <= 100) return raw;
  return raw.slice(0, 100).trim();
}

function markServiceDescription(raw: string | undefined): string {
  const base = (raw || '').trim();
  if (base.toLowerCase().includes(DIGITAL_AVATAR_SERVICE_MARKER)) {
    return base;
  }
  if (!base) return DIGITAL_AVATAR_SERVICE_MARKER;
  return `${base}\n\n${DIGITAL_AVATAR_SERVICE_MARKER}`;
}

export function CreateAvatarDialog({
  open,
  onOpenChange,
  teamId,
  managerAgentId,
  managerAgentName,
  onCreated,
}: CreateAvatarDialogProps) {
  const { t } = useTranslation();
  const [allAgents, setAllAgents] = useState<TeamAgent[]>([]);
  const [templateAgents, setTemplateAgents] = useState<TeamAgent[]>([]);
  const [loadingAgents, setLoadingAgents] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');

  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [slugManual, setSlugManual] = useState(false);
  const [description, setDescription] = useState('');
  const [avatarType, setAvatarType] = useState<AvatarType>('external_service');
  const [runMode, setRunMode] = useState<AvatarRunMode>('on_demand');
  const [documentAccessMode, setDocumentAccessMode] = useState<PortalDocumentAccessMode>('read_only');
  const [useManagerTemplateForService, setUseManagerTemplateForService] = useState(false);
  const [serviceTemplateAgentId, setServiceTemplateAgentId] = useState('');

  const managerAgent = useMemo(
    () => allAgents.find(agent => agent.id === managerAgentId) || null,
    [allAgents, managerAgentId]
  );
  const managerTemplateAllowed = useMemo(
    () => Boolean(managerAgentId && templateAgents.some(agent => agent.id === managerAgentId)),
    [managerAgentId, templateAgents]
  );
  const effectiveManagerName = managerAgent?.name || managerAgentName || t('digitalAvatar.labels.unset');

  useEffect(() => {
    if (!open) return;
    let active = true;
    setLoadingAgents(true);
    setError('');
    Promise.all([
      agentApi.listAgents(teamId, 1, 200),
      portalApi.list(teamId, 1, 200, 'avatar'),
    ])
      .then(([agentRes, avatarRes]) => {
        if (!active) return;
        const agents = agentRes.items || [];
        const avatars = avatarRes.items || [];
        const { generalAgents } = splitGeneralAndDedicatedAgents(agents, avatars);
        setAllAgents(agents);
        setTemplateAgents(generalAgents);
        if (!serviceTemplateAgentId) {
          setServiceTemplateAgentId(generalAgents[0]?.id || '');
        }
      })
      .catch((err) => {
        if (!active) return;
        setError(err instanceof Error ? err.message : t('common.error'));
      })
      .finally(() => {
        if (!active) return;
        setLoadingAgents(false);
      });
    return () => {
      active = false;
    };
  }, [open, teamId, t, managerAgentId]);

  useEffect(() => {
    if (slugManual) return;
    setSlug(slugify(name));
  }, [name, slugManual]);

  useEffect(() => {
    if (useManagerTemplateForService && managerAgentId) {
      setServiceTemplateAgentId(managerAgentId);
    }
  }, [managerAgentId, useManagerTemplateForService]);

  useEffect(() => {
    if (managerTemplateAllowed) return;
    if (useManagerTemplateForService) {
      setUseManagerTemplateForService(false);
    }
  }, [managerTemplateAllowed, useManagerTemplateForService]);

  useEffect(() => {
    if (useManagerTemplateForService) return;
    if (!serviceTemplateAgentId) return;
    if (templateAgents.some(agent => agent.id === serviceTemplateAgentId)) return;
    setServiceTemplateAgentId(templateAgents[0]?.id || '');
  }, [serviceTemplateAgentId, templateAgents, useManagerTemplateForService]);

  const resetForm = () => {
    setName('');
    setSlug('');
    setSlugManual(false);
    setDescription('');
    setAvatarType('external_service');
    setRunMode('on_demand');
    setDocumentAccessMode('read_only');
    setUseManagerTemplateForService(false);
    setServiceTemplateAgentId(templateAgents[0]?.id || '');
    setError('');
  };

  const handleCreate = async () => {
    if (!name.trim()) return;
    if (!managerAgentId) {
      setError(t('digitalAvatar.createDialog.managerRequired'));
      return;
    }
    if (!useManagerTemplateForService && !serviceTemplateAgentId) {
      setError(t('digitalAvatar.createDialog.serviceTemplateRequired', '请选择一个通用 Agent 作为分身模板'));
      return;
    }

    const serviceTemplateId = useManagerTemplateForService
      ? managerAgentId
      : serviceTemplateAgentId;

    setSubmitting(true);
    setError('');
    let created = false;
    let serviceDedicated: TeamAgent | null = null;
    try {
      const normalizedSlug = slug.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-');
      const baseName = name.trim();

      serviceDedicated = await agentApi.provisionFromTemplate(serviceTemplateId, {
        name: buildDedicatedServiceAgentName(baseName),
      });
      const nextDescription = markServiceDescription(serviceDedicated.description);
      if ((serviceDedicated.description || '').trim() !== nextDescription) {
        serviceDedicated = await agentApi.updateAgent(serviceDedicated.id, {
          description: nextDescription,
        });
      }

      const req = {
        name: baseName,
        slug: normalizedSlug || undefined,
        description: description.trim() || undefined,
        outputForm: 'agent_only' as const,
        agentEnabled: true,
        codingAgentId: managerAgentId,
        serviceAgentId: serviceDedicated.id,
        documentAccessMode,
        tags: [
          'digital-avatar',
          avatarType === 'external_service' ? 'avatar:external' : 'avatar:internal',
          `manager:${managerAgentId}`,
        ],
        settings: {
          avatarType,
          runMode,
          managerApprovalMode: 'manager_decides',
          optimizationMode: 'dual_loop',
          managerAgentId,
          managerGroupId: managerAgentId,
          serviceTemplateAgentId: serviceTemplateId,
          serviceRuntimeAgentId: serviceDedicated.id,
        },
      };
      const portal = await portalApi.create(teamId, req);
      created = true;
      onCreated(portal);
      onOpenChange(false);
      resetForm();
    } catch (err) {
      if (!created && serviceDedicated?.id) {
        await agentApi.deleteAgent(serviceDedicated.id).catch(() => undefined);
      }
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[560px]">
        <DialogHeader>
          <DialogTitle>{t('digitalAvatar.createDialog.title')}</DialogTitle>
          <DialogDescription>{t('digitalAvatar.createDialog.description')}</DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-1">
          <div className="rounded-md border bg-muted/20 p-2.5">
            <p className="text-caption text-muted-foreground">{t('digitalAvatar.labels.managerAgent')}</p>
            <p className="mt-0.5 text-xs font-medium text-foreground">{effectiveManagerName}</p>
          </div>

          <div>
            <label className="text-xs font-medium">{t('digitalAvatar.createDialog.name')}</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('digitalAvatar.createDialog.namePlaceholder')}
            />
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.slug')}</label>
              <Input
                value={slug}
                onChange={(e) => {
                  setSlugManual(true);
                  setSlug(e.target.value);
                }}
                placeholder={t('digitalAvatar.createDialog.slugPlaceholder')}
              />
            </div>
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.type')}</label>
              <select
                className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                value={avatarType}
                onChange={(e) => setAvatarType(e.target.value as AvatarType)}
              >
                <option value="external_service">{t('digitalAvatar.types.external')}</option>
                <option value="internal_worker">{t('digitalAvatar.types.internal')}</option>
              </select>
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.runMode')}</label>
              <select
                className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                value={runMode}
                onChange={(e) => setRunMode(e.target.value as AvatarRunMode)}
              >
                <option value="on_demand">{t('digitalAvatar.runModes.on_demand')}</option>
                <option value="scheduled">{t('digitalAvatar.runModes.scheduled')}</option>
                <option value="event_driven">{t('digitalAvatar.runModes.event_driven')}</option>
              </select>
            </div>
            <div>
              <label className="text-xs font-medium">{t('digitalAvatar.createDialog.documentAccess')}</label>
              <select
                className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                value={documentAccessMode}
                onChange={(e) => setDocumentAccessMode(e.target.value as PortalDocumentAccessMode)}
              >
                <option value="read_only">{t('laboratory.documentAccessModeReadOnly')}</option>
                <option value="co_edit_draft">{t('laboratory.documentAccessModeCoEditDraft')}</option>
                <option value="controlled_write">{t('laboratory.documentAccessModeControlledWrite')}</option>
              </select>
            </div>
          </div>

          <div className="space-y-2 rounded-md border p-2.5">
            <label className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                checked={useManagerTemplateForService}
                onChange={(e) => setUseManagerTemplateForService(e.target.checked)}
                disabled={!managerTemplateAllowed}
              />
              {t(
                'digitalAvatar.createDialog.useManagerTemplateForService',
                '分身 Agent 直接沿用当前管理 Agent 模板'
              )}
            </label>
            {!managerTemplateAllowed && (
              <p className="text-caption text-muted-foreground">
                {t(
                  'digitalAvatar.createDialog.managerTemplateBlockedHint',
                  '当前管理 Agent 属于分身专用 Agent，按策略不可作为复制模板。请改用通用 Agent 模板。'
                )}
              </p>
            )}

            {!useManagerTemplateForService && (
              <div>
                <label className="text-xs font-medium">{t('digitalAvatar.createDialog.serviceAgent')}</label>
                <select
                  className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
                  value={serviceTemplateAgentId}
                  onChange={(e) => setServiceTemplateAgentId(e.target.value)}
                  disabled={loadingAgents}
                >
                  <option value="">{t('digitalAvatar.createDialog.noAgent')}</option>
                  {templateAgents.map(agent => (
                    <option key={agent.id} value={agent.id}>
                      {agent.name}{agent.model ? ` (${agent.model})` : ''}
                    </option>
                  ))}
                </select>
                {templateAgents.length === 0 && (
                  <p className="mt-1 text-caption text-muted-foreground">
                    {t(
                      'digitalAvatar.createDialog.noGeneralTemplateHint',
                      '暂无可用的通用 Agent 模板，请先在 Agent 频道创建一个通用 Agent。'
                    )}
                  </p>
                )}
              </div>
            )}
            <p className="text-caption text-muted-foreground">
              {t(
                'digitalAvatar.createDialog.serviceAgentHint',
                '系统会基于模板创建独立分身 Agent，不会修改原模板 Agent。'
              )}
            </p>
          </div>

          <div>
            <label className="text-xs font-medium">{t('digitalAvatar.createDialog.descriptionLabel')}</label>
            <Input
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t('digitalAvatar.createDialog.descriptionPlaceholder')}
            />
          </div>

          {error && (
            <p className="text-xs text-[hsl(var(--destructive))]">{error}</p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button onClick={handleCreate} disabled={submitting || !name.trim() || !managerAgentId}>
            {submitting ? t('common.creating') : t('digitalAvatar.createDialog.create')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
