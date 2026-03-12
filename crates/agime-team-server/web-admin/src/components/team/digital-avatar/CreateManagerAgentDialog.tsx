import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../ui/dialog';
import { Button } from '../../ui/button';
import { Input } from '../../ui/input';
import { agentApi, type TeamAgent } from '../../../api/agent';
import { avatarPortalApi } from '../../../api/avatarPortal';
import { splitGeneralAndDedicatedAgents } from '../agentIsolation';

interface CreateManagerAgentDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  onCreated: (agent: TeamAgent) => void;
}

function isManagerCandidate(agent: TeamAgent): boolean {
  const enabled = (agent.enabled_extensions || [])
    .filter(item => item.enabled)
    .map(item => item.extension);
  return enabled.includes('developer') || enabled.includes('extension_manager') || enabled.includes('team');
}

function buildManagerAgentName(seed: string): string {
  const cleaned = seed.trim() || '数字分身';
  const name = `${cleaned} - 管理Agent`;
  if (name.length <= 100) return name;
  return name.slice(0, 100).trim();
}

export function CreateManagerAgentDialog({
  open,
  onOpenChange,
  teamId,
  onCreated,
}: CreateManagerAgentDialogProps) {
  const { t } = useTranslation();
  const [templates, setTemplates] = useState<TeamAgent[]>([]);
  const [loadingTemplates, setLoadingTemplates] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [templateAgentId, setTemplateAgentId] = useState('');
  const [name, setName] = useState('');
  const [error, setError] = useState('');

  const managerTemplates = useMemo(() => {
    const preferred = templates.filter(isManagerCandidate);
    return preferred.length > 0 ? preferred : templates;
  }, [templates]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    setLoadingTemplates(true);
    setError('');
    Promise.all([
      agentApi.listAgents(teamId, 1, 200),
      avatarPortalApi.list(teamId, 1, 200),
    ])
      .then(([agentRes, avatarRes]) => {
        if (!active) return;
        const allAgents = agentRes.items || [];
        const avatars = avatarRes.items || [];
        const { generalAgents } = splitGeneralAndDedicatedAgents(allAgents, avatars);
        setTemplates(generalAgents);
        const defaultTemplate = generalAgents.find(isManagerCandidate) || generalAgents[0];
        if (defaultTemplate) {
          setTemplateAgentId(defaultTemplate.id);
          setName(buildManagerAgentName(defaultTemplate.name));
        } else {
          setTemplateAgentId('');
          setName(buildManagerAgentName('数字分身'));
        }
      })
      .catch((err) => {
        if (!active) return;
        setError(err instanceof Error ? err.message : t('common.error'));
      })
      .finally(() => {
        if (!active) return;
        setLoadingTemplates(false);
      });
    return () => {
      active = false;
    };
  }, [open, teamId, t]);

  const handleTemplateChange = (nextId: string) => {
    setTemplateAgentId(nextId);
    const matched = templates.find(item => item.id === nextId);
    if (!matched) return;
    setName((prev) => {
      if (prev.trim().length > 0) return prev;
      return buildManagerAgentName(matched.name);
    });
  };

  const handleCreate = async () => {
    if (!templateAgentId) {
      setError(t('digitalAvatar.managerDialog.templateRequired'));
      return;
    }
    setSubmitting(true);
    setError('');
    try {
      const finalAgent = await agentApi.provisionFromTemplate(templateAgentId, {
        name: buildManagerAgentName(name),
        agent_domain: 'digital_avatar',
        agent_role: 'manager',
        template_source_agent_id: templateAgentId,
      });
      onCreated(finalAgent);
      onOpenChange(false);
      setTemplateAgentId('');
      setName('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>{t('digitalAvatar.managerDialog.title', '新建管理 Agent')}</DialogTitle>
          <DialogDescription>
            {t('digitalAvatar.managerDialog.description', '先创建一个专用管理 Agent 组，再在该管理组下创建多个分身。')}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-1">
          <div>
            <label className="text-xs font-medium">
              {t('digitalAvatar.managerDialog.templateLabel', '管理 Agent 模板')}
            </label>
            <select
              className="mt-1 h-9 w-full rounded-md border bg-background px-2.5 text-sm"
              value={templateAgentId}
              onChange={(e) => handleTemplateChange(e.target.value)}
              disabled={loadingTemplates}
            >
              <option value="">{t('digitalAvatar.createDialog.noAgent')}</option>
              {managerTemplates.map(agent => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}{agent.model ? ` (${agent.model})` : ''}
                </option>
              ))}
            </select>
            {managerTemplates.length > 0 ? (
              <p className="mt-1 text-caption text-muted-foreground">
                {t('digitalAvatar.managerDialog.templateHint', '只复制模板配置，不会修改原模板 Agent。')}
              </p>
            ) : (
              <p className="mt-1 text-caption text-muted-foreground">
                {t('digitalAvatar.managerDialog.noGeneralTemplateHint', '暂无可用的通用 Agent 模板，请先在 Agent 频道创建一个通用 Agent。')}
              </p>
            )}
          </div>

          <div>
            <label className="text-xs font-medium">
              {t('digitalAvatar.managerDialog.nameLabel', '管理组名称')}
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t('digitalAvatar.managerDialog.namePlaceholder', '例如：客服分身管理组')}
            />
          </div>

          {error && <p className="text-xs text-[hsl(var(--destructive))]">{error}</p>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button onClick={handleCreate} disabled={submitting || !templateAgentId}>
            {submitting
              ? t('common.creating')
              : t('digitalAvatar.managerDialog.createButton', '创建管理 Agent')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}



