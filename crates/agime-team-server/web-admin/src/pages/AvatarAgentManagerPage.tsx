import { useEffect, useMemo, useState } from 'react';
import { Link, useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, Bot, ChevronDown, ChevronRight, FileText, MessageSquareText, Pencil, Trash2 } from 'lucide-react';
import { AppShell } from '../components/layout/AppShell';
import { Button } from '../components/ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../components/ui/card';
import { Badge } from '../components/ui/badge';
import { Skeleton } from '../components/ui/skeleton';
import { Input } from '../components/ui/input';
import { Textarea } from '../components/ui/textarea';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../components/ui/dialog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../components/ui/select';
import { ConfirmDialog } from '../components/ui/confirm-dialog';
import { StatusBadge, AGENT_STATUS_MAP } from '../components/ui/status-badge';
import { AgentTypeBadge } from '../components/agent/AgentTypeBadge';
import { TeamProvider } from '../contexts/TeamContext';
import {
  buildAvatarPublicNarrativePayload,
  joinNarrativeUseCases,
  readAvatarPublicNarrative,
  splitNarrativeUseCases,
} from '../lib/avatarPublicNarrative';
import { CreateInviteDialog } from '../components/team/CreateInviteDialog';
import { EditAgentDialog } from '../components/agent/EditAgentDialog';
import { DeleteAgentDialog } from '../components/agent/DeleteAgentDialog';
import { AgentDocumentPanel } from '../components/agent/AgentDocumentPanel';
import { DocumentPicker } from '../components/documents/DocumentPicker';
import { apiClient } from '../api/client';
import type { TeamWithStats } from '../api/types';
import { BUILTIN_EXTENSIONS, agentApi, type TeamAgent } from '../api/agent';
import { documentApi, type DocumentSummary } from '../api/documents';
import {
  avatarPortalApi,
  type PortalDetail,
  type PortalDocumentAccessMode,
  type PortalSummary,
  type UpdatePortalRequest,
} from '../api/avatarPortal';
import {
  UNGROUPED_MANAGER_KEY,
  buildDedicatedAvatarGrouping,
  getDigitalAvatarServiceId,
  isDedicatedAvatarService,
  type DedicatedAvatarGroup,
  type DedicatedAvatarPortalLink,
} from '../components/team/agentIsolation';

function getEnabledExtensionNames(agent: TeamAgent): string[] {
  const enabled = agent.enabled_extensions?.filter(ext => ext.enabled) || [];
  return enabled.map(ext => {
    const builtin = BUILTIN_EXTENSIONS.find(item => item.id === ext.extension);
    return builtin?.name || ext.extension;
  });
}

function getEnabledExtensionEntries(agent: TeamAgent): Array<{ id: string; name: string }> {
  return (agent.enabled_extensions || [])
    .filter(ext => ext.enabled)
    .map(ext => {
      const builtin = BUILTIN_EXTENSIONS.find(item => item.id === ext.extension);
      return {
        id: ext.extension,
        name: builtin?.name || ext.extension,
      };
    });
}

function getAssignedSkillEntries(agent: TeamAgent): Array<{ id: string; name: string }> {
  return (agent.assigned_skills || [])
    .filter(skill => skill.enabled)
    .map(skill => ({
      id: skill.skill_id,
      name: skill.name || skill.skill_id,
    }));
}

function normalizeStringSelection(items: string[]): string[] {
  return Array.from(new Set(items)).sort((a, b) => a.localeCompare(b));
}

function sameStringSelection(left: string[], right: string[]): boolean {
  const a = normalizeStringSelection(left);
  const b = normalizeStringSelection(right);
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

async function listAllTeamAgents(teamId: string, pageSize = 200): Promise<TeamAgent[]> {
  const firstPage = await agentApi.listAgents(teamId, 1, pageSize);
  const totalPages = Math.max(firstPage.total_pages || 1, 1);
  const pages = [firstPage];
  for (let page = 2; page <= totalPages; page += 1) {
    pages.push(await agentApi.listAgents(teamId, page, pageSize));
  }
  const dedup = new Map<string, TeamAgent>();
  for (const page of pages) {
    for (const item of page.items || []) {
      dedup.set(item.id, item);
    }
  }
  return Array.from(dedup.values());
}

async function listAllAvatarPortals(teamId: string, pageSize = 200): Promise<PortalSummary[]> {
  const firstPage = await avatarPortalApi.list(teamId, 1, pageSize);
  const totalPages = Math.max(firstPage.totalPages || 1, 1);
  const pages = [firstPage];
  for (let page = 2; page <= totalPages; page += 1) {
    pages.push(await avatarPortalApi.list(teamId, page, pageSize));
  }
  const dedup = new Map<string, PortalSummary>();
  for (const page of pages) {
    for (const item of page.items || []) {
      dedup.set(item.id, item);
    }
  }
  return Array.from(dedup.values());
}

function resolveExtensionNames(
  extensionIds: string[],
  fallbackEntries: Array<{ id: string; name: string }>,
): string[] {
  return extensionIds.map(id => {
    const matched = fallbackEntries.find(item => item.id === id);
    if (matched) {
      return matched.name;
    }
    const builtin = BUILTIN_EXTENSIONS.find(item => item.id === id);
    return builtin?.name || id;
  });
}

function formatDocumentAccessMode(
  mode: PortalDetail['documentAccessMode'] | undefined,
  translate: (key: string, fallback: string) => string,
): string {
  switch (mode) {
    case 'read_only':
      return translate('ecosystem.documentAccessModeReadOnly', '只读');
    case 'co_edit_draft':
      return translate('ecosystem.documentAccessModeCoEditDraft', '协作草稿');
    case 'controlled_write':
      return translate('ecosystem.documentAccessModeControlledWrite', '受控写入');
    default:
      return translate('common.notSet', '未设置');
  }
}

function formatDocumentAccessHint(
  mode: PortalDetail['documentAccessMode'] | undefined,
  translate: (key: string, fallback: string) => string,
): string {
  switch (mode) {
    case 'read_only':
      return translate('ecosystem.documentAccessModeReadOnlyHint', '访客仅可读取/检索/列出绑定文档。');
    case 'co_edit_draft':
      return translate('ecosystem.documentAccessModeCoEditDraftHint', '访客可创建文档，并继续修改与绑定文档相关的 Agent 草稿。');
    case 'controlled_write':
      return translate('ecosystem.documentAccessModeControlledWriteHint', '访客可直接写入目标文档，也可继续修改相关 AI 文档。');
    default:
      return translate('agent.manage.noneConfigured', '未配置');
  }
}

function renderChipList(items: string[], emptyLabel: string) {
  if (items.length === 0) {
    return <span className="text-sm text-muted-foreground">{emptyLabel}</span>;
  }

  return (
    <div className="flex flex-wrap gap-2">
      {items.map(item => (
        <Badge key={item} variant="secondary" className="text-[11px]">
          {item}
        </Badge>
      ))}
    </div>
  );
}

function buildPermissionPreview(
  mode: PortalDetail['documentAccessMode'] | undefined,
  t: (key: string, fallback: string) => string,
): string[] {
  return [
    mode === 'read_only'
      ? t('ecosystem.permissionPreviewRead', '读取 / 列出 / 检索绑定文档')
      : t('ecosystem.permissionPreviewReadWrite', '读取 / 列出 / 检索绑定文档，并可创建文档'),
    mode === 'co_edit_draft'
      ? t('ecosystem.permissionPreviewDraftOnly', '更新仅限与绑定文档相关的 Agent 草稿')
      : mode === 'read_only'
      ? t('ecosystem.permissionPreviewNoUpdate', '文档更新已禁用')
      : t('ecosystem.permissionPreviewControlledWrite', '文档可直接更新，并保留 AI 版本链路'),
    t('ecosystem.permissionPreviewBoundDocs', '仍受绑定文档范围约束'),
  ];
}

function ToggleChipGroup({
  items,
  selectedIds,
  emptyLabel,
  onToggle,
}: {
  items: Array<{ id: string; name: string }>;
  selectedIds: string[];
  emptyLabel: string;
  onToggle: (id: string) => void;
}) {
  if (items.length === 0) {
    return <span className="text-sm text-muted-foreground">{emptyLabel}</span>;
  }

  return (
    <div className="flex flex-wrap gap-2">
      {items.map(item => {
        const selected = selectedIds.includes(item.id);
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onToggle(item.id)}
            className={[
              'rounded-full border px-3 py-1 text-xs transition-colors',
              selected
                ? 'border-primary/40 bg-primary/10 text-primary'
                : 'border-border bg-background text-muted-foreground hover:border-primary/30 hover:text-foreground',
            ].join(' ')}
          >
            {item.name}
          </button>
        );
      })}
    </div>
  );
}

function AgentActionBar({
  agent,
  onEdit,
  onDelete,
  onChat,
  editLabel,
  deleteLabel,
  chatLabel,
}: {
  agent: TeamAgent;
  onEdit: (agent: TeamAgent) => void;
  onDelete: (agent: TeamAgent) => void;
  onChat: (agent: TeamAgent) => void;
  editLabel: string;
  deleteLabel: string;
  chatLabel: string;
}) {
  return (
    <div className="grid w-full grid-cols-1 gap-2 sm:flex sm:flex-wrap sm:items-center">
      <Button size="sm" variant="outline" className="w-full sm:w-auto" onClick={() => onEdit(agent)}>
        <Pencil className="mr-1.5 h-3.5 w-3.5" />
        {editLabel}
      </Button>
      <Button size="sm" variant="outline" className="w-full sm:w-auto" onClick={() => onDelete(agent)}>
        <Trash2 className="mr-1.5 h-3.5 w-3.5" />
        {deleteLabel}
      </Button>
      <Button size="sm" className="w-full sm:w-auto" onClick={() => onChat(agent)}>
        <MessageSquareText className="mr-1.5 h-3.5 w-3.5" />
        {chatLabel}
      </Button>
    </div>
  );
}

function EditAvatarDialog({
  teamId,
  portal,
  serviceAgent,
  documentsById,
  open,
  onOpenChange,
  onSaved,
}: {
  teamId: string;
  portal: PortalDetail | null;
  serviceAgent: TeamAgent | null;
  documentsById: Map<string, DocumentSummary>;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => Promise<void> | void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [slug, setSlug] = useState('');
  const [description, setDescription] = useState('');
  const [heroIntro, setHeroIntro] = useState('');
  const [heroUseCasesText, setHeroUseCasesText] = useState('');
  const [heroWorkingStyle, setHeroWorkingStyle] = useState('');
  const [heroCtaHint, setHeroCtaHint] = useState('');
  const [documentAccessMode, setDocumentAccessMode] = useState<PortalDocumentAccessMode>('read_only');
  const [boundDocumentIds, setBoundDocumentIds] = useState<string[]>([]);
  const [selectedExtensions, setSelectedExtensions] = useState<string[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);
  const [extensionsDirty, setExtensionsDirty] = useState(false);
  const [skillsDirty, setSkillsDirty] = useState(false);
  const [selectedDocumentMap, setSelectedDocumentMap] = useState<Map<string, DocumentSummary>>(new Map());
  const [pickerOpen, setPickerOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const extensionOptions = useMemo(
    () => (serviceAgent ? getEnabledExtensionEntries(serviceAgent) : []),
    [serviceAgent]
  );
  const skillOptions = useMemo(
    () => (serviceAgent ? getAssignedSkillEntries(serviceAgent) : []),
    [serviceAgent]
  );
  const selectedDocuments = boundDocumentIds
    .map(id => selectedDocumentMap.get(id))
    .filter((doc): doc is DocumentSummary => Boolean(doc));

  useEffect(() => {
    if (!open || !portal) return;
    const narrative = readAvatarPublicNarrative(portal.settings);
    setName(portal.name || '');
    setSlug(portal.slug || '');
    setDescription(portal.description || '');
    setHeroIntro(narrative.heroIntro || '');
    setHeroUseCasesText(joinNarrativeUseCases(narrative.heroUseCases));
    setHeroWorkingStyle(narrative.heroWorkingStyle || '');
    setHeroCtaHint(narrative.heroCtaHint || '');
    setDocumentAccessMode(portal.documentAccessMode || 'read_only');
    setBoundDocumentIds(portal.boundDocumentIds || []);
    setSelectedDocumentMap(new Map(documentsById));
    setSelectedExtensions(portal.allowedExtensions ?? extensionOptions.map(item => item.id));
    setSelectedSkillIds(portal.allowedSkillIds ?? skillOptions.map(item => item.id));
    setExtensionsDirty(false);
    setSkillsDirty(false);
    setError('');
  }, [documentsById, extensionOptions, open, portal, skillOptions]);

  const toggleSelection = (value: string, selected: string[], setSelected: (value: string[]) => void) => {
    setSelected(
      selected.includes(value)
        ? selected.filter(item => item !== value)
        : [...selected, value]
    );
  };

  const handleSave = async () => {
    if (!portal) return;
    const nextName = name.trim();
    const nextSlug = slug.trim();
    if (!nextName || !nextSlug) {
      setError(t('agent.manage.avatarEditValidation', '分身名称和访问地址不能为空。'));
      return;
    }

    try {
      setSaving(true);
      setError('');
      const avatarPublicNarrative = buildAvatarPublicNarrativePayload({
        heroIntro,
        heroUseCases: splitNarrativeUseCases(heroUseCasesText),
        heroWorkingStyle,
        heroCtaHint,
      });
      const nextSettings: Record<string, unknown> = { ...(portal.settings || {}) };
      if (avatarPublicNarrative) {
        nextSettings.avatarPublicNarrative = avatarPublicNarrative;
      } else {
        delete nextSettings.avatarPublicNarrative;
      }
      const req: UpdatePortalRequest = {
        name: nextName,
        slug: nextSlug,
        description: description.trim() || '',
        documentAccessMode,
        boundDocumentIds,
        settings: nextSettings,
      };
      if (extensionsDirty) {
        const inheritedExtensions = extensionOptions.map(item => item.id);
        const currentExtensions = portal.allowedExtensions ?? inheritedExtensions;
        if (!sameStringSelection(selectedExtensions, currentExtensions)) {
          req.allowedExtensions = selectedExtensions;
        }
      }
      if (skillsDirty) {
        const inheritedSkills = skillOptions.map(item => item.id);
        const currentSkills = portal.allowedSkillIds ?? inheritedSkills;
        if (!sameStringSelection(selectedSkillIds, currentSkills)) {
          req.allowedSkillIds = selectedSkillIds;
        }
      }
      await avatarPortalApi.update(teamId, portal.id, req);
      onOpenChange(false);
      await onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="max-w-4xl">
          <DialogHeader>
            <DialogTitle>{t('agent.manage.editAvatarTitle', '编辑分身')}</DialogTitle>
            <DialogDescription>
              {t('agent.manage.editAvatarDescription', '这里只修改分身本身的对外名称、权限边界和文档绑定，不修改底层服务 Agent。')}
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('common.name', '名称')}
              </label>
              <Input value={name} onChange={(e) => setName(e.target.value)} />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('agent.manage.avatarSlugLabel', '访问地址')}
              </label>
              <Input value={slug} onChange={(e) => setSlug(e.target.value.replace(/^\//, ''))} />
            </div>
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium text-foreground">
              {t('common.description', '描述')}
            </label>
            <Textarea
              rows={3}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder={t('agent.manage.avatarDescriptionPlaceholder', 'Explain what this avatar handles and what kinds of problems it is suitable for.')}
            />
          </div>

          <div className="space-y-3 rounded-lg border border-border/70 bg-muted/10 p-4">
            <div>
              <h3 className="text-sm font-medium text-foreground">
                {t('agent.manage.avatarNarrativeTitle', '公开页顶部叙事')}
              </h3>
              <p className="mt-1 text-xs leading-5 text-muted-foreground">
                {t(
                  'agent.manage.avatarNarrativeHint',
                  'This content appears at the top of the public avatar page and explains why the avatar exists, what it is good at, and how users should begin.',
                )}
              </p>
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('agent.manage.avatarNarrativeIntroLabel', '顶部主叙事')}
              </label>
              <Textarea
                rows={3}
                value={heroIntro}
                onChange={(e) => setHeroIntro(e.target.value)}
                placeholder={t(
                  'agent.manage.avatarNarrativeIntroPlaceholder',
                  'For example: this is a service avatar for customer support, specialized in locating issues quickly from product materials, organizing answers, and suggesting next steps.',
                )}
              />
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('agent.manage.avatarNarrativeUseCasesLabel', '典型任务（每行一条）')}
              </label>
              <Textarea
                rows={4}
                value={heroUseCasesText}
                onChange={(e) => setHeroUseCasesText(e.target.value)}
                placeholder={t(
                  'agent.manage.avatarNarrativeUseCasesPlaceholder',
                  'Answer product usage questions\nCreate plans from materials\nContinue working on a specified document',
                )}
              />
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <label className="text-sm font-medium text-foreground">
                  {t('agent.manage.avatarNarrativeWorkingStyleLabel', '处理方式说明')}
                </label>
                <Textarea
                  rows={3}
                  value={heroWorkingStyle}
                  onChange={(e) => setHeroWorkingStyle(e.target.value)}
                  placeholder={t(
                    'agent.manage.avatarNarrativeWorkingStylePlaceholder',
                    'For example: I first work from currently available materials; if the request is out of scope, I escalate it to the managing agent.',
                  )}
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium text-foreground">
                  {t('agent.manage.avatarNarrativeCtaHintLabel', '开始提示')}
                </label>
                <Textarea
                  rows={3}
                  value={heroCtaHint}
                  onChange={(e) => setHeroCtaHint(e.target.value)}
                  placeholder={t(
                    'agent.manage.avatarNarrativeCtaHintPlaceholder',
                    'For example: describe the issue directly in the chat channel; if you want me to work with materials, select the target document in the documents channel first.',
                  )}
                />
              </div>
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('ecosystem.documentAccessMode', '文档访问模式')}
              </label>
              <Select value={documentAccessMode} onValueChange={(value) => setDocumentAccessMode(value as PortalDocumentAccessMode)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="read_only">{t('ecosystem.documentAccessModeReadOnly', '只读')}</SelectItem>
                  <SelectItem value="co_edit_draft">{t('ecosystem.documentAccessModeCoEditDraft', '协作草稿')}</SelectItem>
                  <SelectItem value="controlled_write">{t('ecosystem.documentAccessModeControlledWrite', '受控写入')}</SelectItem>
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {formatDocumentAccessHint(documentAccessMode, (key, fallback) => t(key, fallback))}
              </p>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <label className="text-sm font-medium text-foreground">
                  {t('agent.manage.avatarBoundDocumentsLabel', '绑定文档')}
                </label>
                <Button size="sm" variant="outline" onClick={() => setPickerOpen(true)}>
                  {t('documents.selectDocuments', '选择文档')}
                </Button>
              </div>
              {selectedDocuments.length > 0 ? (
                <div className="flex flex-wrap gap-2 rounded-lg border border-border/60 bg-muted/10 p-3">
                  {selectedDocuments.map(doc => (
                    <Badge key={doc.id} variant="secondary" className="text-[11px]">
                      {doc.display_name || doc.name}
                    </Badge>
                  ))}
                </div>
              ) : (
                <div className="rounded-lg border border-dashed border-border/70 px-3 py-4 text-sm text-muted-foreground">
                  {t('agent.manage.noBoundDocuments', '未绑定文档')}
                </div>
              )}
            </div>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('ecosystem.allowedExtensionsVisitor', '允许的扩展（访客）')}
              </label>
              <ToggleChipGroup
                items={extensionOptions}
                selectedIds={selectedExtensions}
                emptyLabel={t('ecosystem.noEnabledExtensionsOnAgent', '底层服务 Agent 没有可用扩展')}
                onToggle={(value) => {
                  setExtensionsDirty(true);
                  toggleSelection(value, selectedExtensions, setSelectedExtensions);
                }}
              />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('ecosystem.allowedSkillsVisitor', '允许的技能（访客）')}
              </label>
              <ToggleChipGroup
                items={skillOptions}
                selectedIds={selectedSkillIds}
                emptyLabel={t('ecosystem.noAssignedSkillsOnAgent', '底层服务 Agent 没有已分配技能')}
                onToggle={(value) => {
                  setSkillsDirty(true);
                  toggleSelection(value, selectedSkillIds, setSelectedSkillIds);
                }}
              />
            </div>
          </div>

          {error ? <div className="text-sm text-destructive">{error}</div> : null}

          <DialogFooter>
            <Button variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
              {t('common.cancel')}
            </Button>
            <Button onClick={handleSave} disabled={saving}>
              {saving ? t('common.saving', '保存中...') : t('common.save')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <DocumentPicker
        teamId={teamId}
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onSelect={(docs) => {
          setSelectedDocumentMap(prev => {
            const next = new Map(prev);
            docs.forEach(doc => next.set(doc.id, doc));
            return next;
          });
          setBoundDocumentIds(docs.map(doc => doc.id));
          setPickerOpen(false);
        }}
        multiple
        selectedIds={boundDocumentIds}
        selectedDocuments={selectedDocuments}
      />
    </>
  );
}

function ServiceAvatarRow({
  teamId,
  portalLink,
  portalDetail,
  documentsById,
  canManage,
  onEditAvatar,
  onDeleteAvatar,
  onDeleteServiceAgent,
  onEditServiceAgent,
  onChat,
}: {
  teamId: string;
  portalLink: DedicatedAvatarPortalLink;
  portalDetail: PortalDetail | null;
  documentsById: Map<string, DocumentSummary>;
  canManage: boolean;
  onEditAvatar: (portal: PortalDetail) => void;
  onDeleteAvatar: (portalLink: DedicatedAvatarPortalLink) => void;
  onDeleteServiceAgent: (agent: TeamAgent) => void;
  onEditServiceAgent: (agent: TeamAgent) => void;
  onChat: (agent: TeamAgent) => void;
}) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [showAdvancedConfig, setShowAdvancedConfig] = useState(false);
  const [documentPanelOpen, setDocumentPanelOpen] = useState(false);
  const isOrphanService = portalLink.linkKind === 'orphan_service';
  const serviceAgent = portalLink.serviceAgent;
  const enabledExtensionEntries = serviceAgent ? getEnabledExtensionEntries(serviceAgent) : [];
  const enabledExtensionNames = enabledExtensionEntries.map(item => item.name);
  const assignedSkillEntries = serviceAgent ? getAssignedSkillEntries(serviceAgent) : [];
  const assignedSkillNames = assignedSkillEntries.map(item => item.name);
  const boundDocumentEntries = (portalDetail?.boundDocumentIds || [])
    .map(id => documentsById.get(id))
    .filter((doc): doc is DocumentSummary => Boolean(doc));
  const boundDocuments = (portalDetail?.boundDocumentIds || [])
    .map(id => documentsById.get(id)?.display_name || documentsById.get(id)?.name || id);
  const hasPortalCapabilityRestriction = Boolean(
    (portalDetail?.allowedExtensions && portalDetail.allowedExtensions.length > 0) ||
    (portalDetail?.allowedSkillIds && portalDetail.allowedSkillIds.length > 0),
  );
  const visitorAllowedExtensionIds =
    portalDetail?.allowedExtensions && portalDetail.allowedExtensions.length > 0
      ? portalDetail.allowedExtensions
      : enabledExtensionEntries.map(item => item.id);
  const visitorAllowedExtensions = resolveExtensionNames(visitorAllowedExtensionIds, enabledExtensionEntries);
  const visitorAllowedSkillIds =
    portalDetail?.allowedSkillIds && portalDetail.allowedSkillIds.length > 0
      ? portalDetail.allowedSkillIds
      : assignedSkillEntries.map(item => item.id);
  const visitorAllowedSkills = visitorAllowedSkillIds.map(skillId => {
    const matched = assignedSkillEntries.find(skill => skill.id === skillId);
    return matched?.name || skillId;
  });
  const effectiveExtensions = enabledExtensionEntries
    .filter(item => visitorAllowedExtensionIds.includes(item.id))
    .map(item => item.name);
  const effectiveSkills = assignedSkillEntries
    .filter(item => visitorAllowedSkillIds.includes(item.id))
    .map(item => item.name);
  const permissionPreview = buildPermissionPreview(portalDetail?.documentAccessMode, (key, fallback) => t(key, fallback));
  const allowedGroups = serviceAgent?.allowed_groups || [];
  const visitorCapabilityScopeHint = hasPortalCapabilityRestriction
    ? t('digitalAvatar.workspace.capabilityScopeRestricted', '当前已按门户白名单收敛，只开放这里列出的扩展与技能。')
    : t('digitalAvatar.workspace.capabilityScopeInherited', '当前未额外收敛，按服务分身已启用的扩展与技能对外开放。');
  const handleStartDocumentChat = (targetDoc: DocumentSummary, sourceDoc: DocumentSummary | null) => {
    if (!serviceAgent) {
      return;
    }
    const attachedDocumentIds = Array.from(
      new Set(
        [
          targetDoc.id,
          sourceDoc && sourceDoc.id !== targetDoc.id ? sourceDoc.id : null,
        ].filter((id): id is string => Boolean(id))
      )
    );
    const composeText = sourceDoc && sourceDoc.id !== targetDoc.id
      ? `Please continue editing the attached AI document "${targetDoc.display_name || targetDoc.name}". It originates from the original document "${sourceDoc.display_name || sourceDoc.name}". Use "${targetDoc.display_name || targetDoc.name}" as the main target for this session, and treat the original document as reference only. Before starting, confirm which target document you are going to operate on.`
      : `Please continue from the attached document "${targetDoc.display_name || targetDoc.name}" and treat it as the main target for this session. Before starting, confirm which target document you are going to operate on. If changes are needed, execute them around this document.`;

    navigate(`/teams/${teamId}?section=chat&agentId=${serviceAgent.id}`, {
      state: {
        chatLaunchContext: {
          requestId: `agent-doc-chat-${Date.now()}-${targetDoc.id}`,
          attachedDocumentIds,
          composeRequest: {
            id: `agent-doc-compose-${Date.now()}-${targetDoc.id}`,
            text: composeText,
          },
        },
      },
    });
  };

  return (
    <div className="rounded-xl border border-border/70 bg-background px-4 py-4">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 flex-1 space-y-2">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold text-foreground">{portalLink.portalName}</span>
            {isOrphanService ? (
              <Badge variant="outline" className="text-[11px] border-[hsl(var(--status-warning-text))/0.25] bg-[hsl(var(--status-warning-bg))/0.72] text-[hsl(var(--status-warning-text))]">
                {t('agent.manage.orphanServiceBadge', '仅剩服务 Agent')}
              </Badge>
            ) : (
              <Badge variant="secondary" className="text-[11px]">
                {t(`ecosystem.status.${portalLink.portalStatus}`, portalLink.portalStatus)}
              </Badge>
            )}
            {!isOrphanService && portalLink.portalSlug && (
              <Badge variant="outline" className="text-[11px]">
                /{portalLink.portalSlug}
              </Badge>
            )}
          </div>
          {isOrphanService ? (
            <p className="text-xs leading-5 text-muted-foreground">
                {t(
                  'agent.manage.orphanServiceHint',
                  'The avatar entry has already been deleted. What remains is a dedicated service agent without any bound public entry. You can still chat, edit it, or clean it up directly.'
                )}
              </p>
            ) : null}
          <div className="grid gap-2 text-sm text-muted-foreground md:grid-cols-2">
            <div>
              <span className="font-medium text-foreground">
                {t('agent.manage.dedicatedServiceAgentLabel', '服务 Agent')}
              </span>
              <span className="ml-2">
                {serviceAgent?.name || t('agent.manage.missingServiceAgent', '未找到绑定的服务 Agent')}
              </span>
              {serviceAgent && (
                <span className="ml-2 align-middle">
                  <StatusBadge status={AGENT_STATUS_MAP[serviceAgent.status] || 'neutral'} className="text-[10px]">
                    {t(`agent.status.${serviceAgent.status}`)}
                  </StatusBadge>
                </span>
              )}
            </div>
            <div>
              <span className="font-medium text-foreground">{t('agent.model', '模型')}</span>
              <span className="ml-2">{serviceAgent?.model || '-'}</span>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {isOrphanService ? (
            <>
              <Button
                size="sm"
                variant="outline"
                onClick={() => serviceAgent && onEditServiceAgent(serviceAgent)}
                disabled={!serviceAgent}
              >
                <Pencil className="mr-1.5 h-3.5 w-3.5" />
                {t('agent.manage.editServiceAgent', '编辑底层服务 Agent')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => serviceAgent && onDeleteServiceAgent(serviceAgent)}
                disabled={!serviceAgent}
              >
                <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                {t('agent.manage.deleteServiceAgentOnly', '删除服务 Agent')}
              </Button>
            </>
          ) : (
            <>
              <Button
                size="sm"
                variant="outline"
                onClick={() => portalDetail && onEditAvatar(portalDetail)}
                disabled={!portalDetail}
              >
                <Pencil className="mr-1.5 h-3.5 w-3.5" />
                {t('agent.actions.edit')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => portalDetail && onDeleteAvatar(portalLink)}
                disabled={!portalDetail}
              >
                <Trash2 className="mr-1.5 h-3.5 w-3.5" />
                {t('common.delete')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => setDocumentPanelOpen(true)}
                disabled={!portalDetail}
              >
                <FileText className="mr-1.5 h-3.5 w-3.5" />
                {t('agent.manage.openAgentDocuments', '查看文档')}
                {portalDetail ? (
                  <span className="ml-1 rounded-full bg-muted px-1.5 py-0.5 text-[10px] leading-none text-muted-foreground">
                    {portalDetail.boundDocumentIds.length}
                  </span>
                ) : null}
              </Button>
            </>
          )}
          {serviceAgent ? (
            <Button size="sm" onClick={() => onChat(serviceAgent)}>
              <MessageSquareText className="mr-1.5 h-3.5 w-3.5" />
              {t('agent.chat.button')}
            </Button>
          ) : null}
        </div>
      </div>

      <div className="mt-4 rounded-xl border border-border/60 bg-muted/10 p-4">
        <div className="mb-3 text-sm font-medium text-foreground">
          {isOrphanService
            ? t('agent.manage.orphanServiceCapabilityTitle', '当前保留的服务 Agent 能力')
            : t('agent.manage.avatarEffectiveCapabilityTitle', '分身当前生效能力')}
        </div>
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">
              {isOrphanService
                ? t('agent.manage.orphanServiceBindingLabel', '入口绑定状态')
                : t('agent.manage.avatarDocumentAccessLabel', '文档访问模式')}
            </div>
            <div className="text-sm font-medium text-foreground">
              {isOrphanService
                ? t('agent.manage.orphanServiceBindingEmpty', '未绑定分身入口')
                : formatDocumentAccessMode(portalDetail?.documentAccessMode, (key, fallback) => t(key, fallback))}
            </div>
            <div className="text-xs leading-5 text-muted-foreground">
              {isOrphanService
                ? t('agent.manage.orphanServiceBindingHint', '这个服务 Agent 仍然保留执行能力，但不再有访客入口、文档边界或公开地址。')
                : formatDocumentAccessHint(portalDetail?.documentAccessMode, (key, fallback) => t(key, fallback))}
            </div>
          </div>
          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">
              {isOrphanService
                ? t('agent.manage.avatarBoundDocumentsLabel', '原入口文档')
                : t('agent.manage.avatarBoundDocumentsLabel', '绑定文档')}
            </div>
            {renderChipList(boundDocuments, t('agent.manage.noBoundDocuments', '未绑定文档'))}
          </div>
          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">
              {t('agent.manage.avatarEffectiveExtensionsLabel', '当前可用扩展')}
            </div>
            {renderChipList(effectiveExtensions, t('agent.manage.noneConfigured', '未配置'))}
          </div>
          <div className="space-y-1">
            <div className="text-xs text-muted-foreground">
              {t('agent.manage.avatarEffectiveSkillsLabel', '当前可用技能')}
            </div>
            {renderChipList(effectiveSkills, t('agent.manage.noneConfigured', '未配置'))}
          </div>
        </div>

        <div className="mt-4 rounded-xl border border-border/50 bg-background/70 p-3">
          <div className="text-xs font-medium text-foreground">
            {t('agent.manage.avatarEffectivePermissionPreviewLabel', '当前权限效果')}
          </div>
          <ul className="mt-2 space-y-1 text-xs text-muted-foreground">
            {permissionPreview.map(item => (
              <li key={item} className="leading-5">
                {item}
              </li>
            ))}
          </ul>
        </div>
      </div>

      <div className="mt-3 rounded-xl border border-border/60 bg-background/80">
        <div className="flex items-center justify-between gap-3 px-4 py-3">
          <div className="flex-1">
            <div className="text-sm font-medium text-foreground">
              {t('agent.manage.avatarAdvancedConfigTitle', '底层服务 Agent 配置（高级）')}
            </div>
            <div className="mt-1 text-xs text-muted-foreground">
              {t('agent.manage.avatarAdvancedConfigHint', '用于排查能力来源、模板配置与执行限制，普通阅读可忽略。')}
            </div>
          </div>
          <div className="flex items-center gap-2">
            {serviceAgent ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={(event) => {
                  event.stopPropagation();
                  onEditServiceAgent(serviceAgent);
                }}
              >
                <Pencil className="mr-1.5 h-3.5 w-3.5" />
                {t('agent.manage.editServiceAgent', '编辑底层服务 Agent')}
              </Button>
            ) : null}
            <Button
              type="button"
              size="icon"
              variant="ghost"
              onClick={() => setShowAdvancedConfig(value => !value)}
              aria-label={showAdvancedConfig ? t('common.collapse', '收起') : t('common.expand', '展开')}
            >
              {showAdvancedConfig ? (
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
            </Button>
          </div>
        </div>

        {showAdvancedConfig && (
          <div className="border-t border-border/60 px-4 py-4">
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
              <div className="rounded-lg border border-border/50 bg-muted/20 px-3 py-2 text-xs leading-5 text-muted-foreground md:col-span-2 xl:col-span-4">
                {visitorCapabilityScopeHint}
              </div>
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t('agent.extensions.enabled')}
                </div>
                {renderChipList(enabledExtensionNames, t('agent.extensions.none', '无'))}
              </div>
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t('agent.manage.avatarAssignedSkillsLabel', '已分配技能')}
                </div>
                {renderChipList(assignedSkillNames, t('agent.manage.noneConfigured', '未配置'))}
              </div>
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t('agent.access.allowedGroups', '允许的用户组')}
                </div>
                {renderChipList(allowedGroups, t('agent.manage.noneConfigured', '未配置'))}
              </div>
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t('agent.access.maxConcurrent', '最大并发任务数')}
                </div>
                <div className="text-sm font-medium text-foreground">
                  {serviceAgent?.max_concurrent_tasks ?? 5}
                </div>
              </div>
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t('ecosystem.allowedExtensionsVisitor', '允许的扩展（访客）')}
                </div>
                {renderChipList(visitorAllowedExtensions, t('agent.manage.noneConfigured', '未配置'))}
              </div>
              <div className="space-y-1">
                <div className="text-xs text-muted-foreground">
                  {t('ecosystem.allowedSkillsVisitor', '允许的技能（访客）')}
                </div>
                {renderChipList(visitorAllowedSkills, t('agent.manage.noneConfigured', '未配置'))}
              </div>
            </div>
          </div>
        )}
      </div>

      {!isOrphanService ? (
        <AgentDocumentPanel
          open={documentPanelOpen}
          onOpenChange={setDocumentPanelOpen}
          teamId={teamId}
          portalName={portalLink.portalName}
          serviceAgentName={serviceAgent?.name}
          documentAccessMode={portalDetail?.documentAccessMode}
          documentIds={portalDetail?.boundDocumentIds || []}
          documents={boundDocumentEntries}
          canManage={canManage}
          onStartChat={handleStartDocumentChat}
          onOpenDocumentsChannel={() => navigate(`/teams/${teamId}?section=documents`)}
        />
      ) : null}
    </div>
  );
}

export default function AvatarAgentManagerPage() {
  const { t } = useTranslation();
  const { teamId, managerId } = useParams<{ teamId: string; managerId: string }>();
  const navigate = useNavigate();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [group, setGroup] = useState<DedicatedAvatarGroup | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [inviteDialogOpen, setInviteDialogOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    const stored = localStorage.getItem('sidebar-collapsed');
    return stored === 'true';
  });
  const [documentsById, setDocumentsById] = useState<Map<string, DocumentSummary>>(new Map());
  const [portalDetailsById, setPortalDetailsById] = useState<Map<string, PortalDetail>>(new Map());
  const [avatarSummaries, setAvatarSummaries] = useState<PortalSummary[]>([]);
  const [editAgentOpen, setEditAgentOpen] = useState(false);
  const [deleteAgentOpen, setDeleteAgentOpen] = useState(false);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(null);
  const [selectedAvatar, setSelectedAvatar] = useState<PortalDetail | null>(null);
  const [avatarDeleteTarget, setAvatarDeleteTarget] = useState<DedicatedAvatarPortalLink | null>(null);
  const [deletingAvatar, setDeletingAvatar] = useState(false);
  const [deleteAvatarAndService, setDeleteAvatarAndService] = useState(false);
  const [deleteAvatarError, setDeleteAvatarError] = useState('');
  const avatarPortalLinks = useMemo(
    () => (group?.portals || []).filter(item => item.linkKind === 'portal'),
    [group]
  );
  const orphanServiceLinks = useMemo(
    () => (group?.portals || []).filter(item => item.linkKind === 'orphan_service'),
    [group]
  );

  const canManage = team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin';

  const loadData = async () => {
    if (!teamId || !managerId) return;
    try {
      setLoading(true);
      const [teamResult, agentResult, avatarResult] = await Promise.all([
        apiClient.getTeam(teamId),
        listAllTeamAgents(teamId),
        listAllAvatarPortals(teamId),
      ]);
      const grouping = buildDedicatedAvatarGrouping(agentResult, avatarResult);
      const matchedGroup = grouping.dedicatedGroups.find(item => item.managerId === managerId) || null;
      const portalIds = (matchedGroup?.portals || [])
        .filter(item => item.linkKind === 'portal')
        .map(item => item.portalId);
      const portalDetails = await Promise.all(
        portalIds.map(async (portalId) => {
          try {
            const detail = await avatarPortalApi.get(teamId, portalId);
            return [portalId, detail] as const;
          } catch {
            return null;
          }
        })
      );
      const nextPortalDetailsById = new Map<string, PortalDetail>();
      for (const entry of portalDetails) {
        if (!entry) continue;
        nextPortalDetailsById.set(entry[0], entry[1]);
      }

      const boundDocumentIds = Array.from(
        new Set(
          Array.from(nextPortalDetailsById.values()).flatMap(detail => detail.boundDocumentIds || [])
        )
      );
      const resolvedDocuments = boundDocumentIds.length > 0
        ? await documentApi.getDocumentsByIds(teamId, boundDocumentIds)
        : [];
      const nextDocumentsById = new Map<string, DocumentSummary>();
      for (const doc of resolvedDocuments) {
        nextDocumentsById.set(doc.id, doc);
      }

      setTeam(teamResult.team);
      setGroup(matchedGroup);
      setAvatarSummaries(avatarResult);
      setPortalDetailsById(nextPortalDetailsById);
      setDocumentsById(nextDocumentsById);
      setSelectedAgent(prev =>
        prev && agentResult.some(agent => agent.id === prev.id) ? prev : null
      );
      setDeleteAgentOpen(prevOpen =>
        prevOpen && selectedAgent && agentResult.some(agent => agent.id === selectedAgent.id) ? prevOpen : false
      );
      setSelectedAvatar(prev =>
        prev && nextPortalDetailsById.has(prev.id) ? nextPortalDetailsById.get(prev.id) || null : null
      );
      setAvatarDeleteTarget(prev =>
        prev && matchedGroup?.portals.some(item => item.portalId === prev.portalId && item.linkKind === 'portal')
          ? prev
          : null
      );
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();
  }, [teamId, managerId]);

  const handleToggleSidebar = () => {
    setSidebarCollapsed(prev => {
      localStorage.setItem('sidebar-collapsed', String(!prev));
      return !prev;
    });
  };

  const handleSectionChange = (section: string) => {
    if (!teamId) return;
    navigate(`/teams/${teamId}?section=${section}`);
  };

  const openAgentChat = (agent: TeamAgent) => {
    if (!teamId) return;
    navigate(`/teams/${teamId}?section=chat&agentId=${agent.id}`);
  };

  const avatarDeleteServiceUsageCount = useMemo(() => {
    const serviceAgentId = avatarDeleteTarget?.serviceAgent?.id;
    if (!serviceAgentId) return 0;
    return avatarSummaries.filter(summary => getDigitalAvatarServiceId(summary) === serviceAgentId).length;
  }, [avatarDeleteTarget, avatarSummaries]);

  const canCleanupDedicatedService = useMemo(() => {
    const serviceAgent = avatarDeleteTarget?.serviceAgent;
    if (!serviceAgent) return false;
    return isDedicatedAvatarService(serviceAgent) && avatarDeleteServiceUsageCount <= 1;
  }, [avatarDeleteServiceUsageCount, avatarDeleteTarget]);

  const handleDeleteAvatar = async () => {
    if (!teamId || !avatarDeleteTarget) return;
    try {
      setDeletingAvatar(true);
      setDeleteAvatarError('');
      await avatarPortalApi.delete(teamId, avatarDeleteTarget.portalId);
      if (deleteAvatarAndService && canCleanupDedicatedService && avatarDeleteTarget.serviceAgent) {
        await agentApi.deleteAgent(avatarDeleteTarget.serviceAgent.id);
      }
      setAvatarDeleteTarget(null);
      setDeleteAvatarAndService(false);
      await loadData();
    } catch (err) {
      setDeleteAvatarError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setDeletingAvatar(false);
    }
  };

  const managerLabel = useMemo(() => {
    if (!group) return '';
    return group.managerAgent?.name || t('agent.manage.ungroupedManagerTitle', '未归类分组');
  }, [group, t]);

  const managerDescription = useMemo(() => {
    if (!group) return '';
    return group.managerAgent?.description?.trim()
      || t('agent.manage.avatarSectionHint', '仅用于数字分身治理与执行，配置调整不影响常规 Agent。');
  }, [group, t]);

  const managerExtensions = useMemo(() => {
    if (!group?.managerAgent) return [];
    return getEnabledExtensionNames(group.managerAgent);
  }, [group]);

  if (loading) {
    return (
      <AppShell className="team-font-cap">
        <div className="space-y-4">
          <Skeleton className="h-12 w-52" />
          <Skeleton className="h-40 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      </AppShell>
    );
  }

  if (!team || error) {
    return (
      <AppShell className="team-font-cap">
        <div className="flex flex-col items-center justify-center gap-4 py-16 text-center">
          <p className="text-[hsl(var(--destructive))]">{error || t('teams.notFound')}</p>
          <Link to={teamId ? `/teams/${teamId}?section=agent` : '/teams'}>
            <Button variant="outline">{t('teams.backToList')}</Button>
          </Link>
        </div>
      </AppShell>
    );
  }

  return (
    <TeamProvider
      value={{
        team,
        canManage: Boolean(canManage),
        activeSection: 'agent',
        onSectionChange: handleSectionChange,
        onInviteClick: () => setInviteDialogOpen(true),
        sidebarCollapsed,
        onToggleSidebar: handleToggleSidebar,
      }}
    >
      <AppShell className="team-font-cap">
        <div className="space-y-6">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <Button
              variant="ghost"
              size="sm"
              className="px-2"
              onClick={() => navigate(`/teams/${teamId}?section=agent`)}
            >
              <ArrowLeft className="mr-1.5 h-4 w-4" />
              {t('agent.manage.backToAgentManage', '返回 Agent 管理')}
            </Button>
          </div>

          {group ? (
            <>
              <Card className="border-border/70">
                <CardHeader className="gap-4">
                  <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                    <div className="min-w-0 space-y-3">
                      <div className="flex flex-wrap items-center gap-2">
                        <CardTitle className="text-xl">{managerLabel}</CardTitle>
                        {group.managerRoles.map(role => (
                          <AgentTypeBadge
                            key={role}
                            type={role === 'manager' ? 'avatar_manager' : 'avatar_service'}
                          />
                        ))}
                          <Badge variant="secondary" className="text-[11px]">
                            {t('agent.manage.dedicatedGroupAvatarCount', '{{count}} 个分身', {
                            count: avatarPortalLinks.length,
                            })}
                          </Badge>
                        {orphanServiceLinks.length > 0 ? (
                          <Badge variant="outline" className="text-[11px] border-[hsl(var(--status-warning-text))/0.2] bg-[hsl(var(--status-warning-bg))/0.55] text-[hsl(var(--status-warning-text))]">
                            {t('agent.manage.orphanServiceCount', '{{count}} 个残留服务 Agent', {
                              count: orphanServiceLinks.length,
                            })}
                          </Badge>
                        ) : null}
                      </div>
                      <p className="max-w-4xl text-sm leading-6 text-muted-foreground">
                        {managerDescription}
                      </p>
                      <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                        <span>
                          {t('agent.model', '模型')}: {group.managerAgent?.model || '-'}
                        </span>
                        {group.managerAgent?.api_url && (
                          <span className="truncate">
                            API: {group.managerAgent.api_url}
                          </span>
                        )}
                      </div>
                    </div>

                    {group.managerAgent ? (
                      <AgentActionBar
                        agent={group.managerAgent}
                        onEdit={(agent) => {
                          setSelectedAgent(agent);
                          setEditAgentOpen(true);
                        }}
                        onDelete={(agent) => {
                          setSelectedAgent(agent);
                          setDeleteAgentOpen(true);
                        }}
                        onChat={openAgentChat}
                        editLabel={t('agent.actions.edit')}
                        deleteLabel={t('common.delete')}
                        chatLabel={t('agent.chat.button')}
                      />
                    ) : null}
                  </div>
                </CardHeader>
                <CardContent className="space-y-5">
                  <div className="grid grid-cols-2 gap-3 md:grid-cols-3">
                    <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4">
                      <div className="text-xs text-muted-foreground">
                        {t('agent.manage.dedicatedGroupAvatarCountLabel', '当前分身数')}
                      </div>
                        <div className="mt-2 text-2xl font-semibold text-foreground">
                        {avatarPortalLinks.length}
                        </div>
                      </div>
                    <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4">
                      <div className="text-xs text-muted-foreground">
                        {t('agent.manage.managerAgentTypeLabel', '管理 Agent 类型')}
                      </div>
                      <div className="mt-2 text-sm font-medium text-foreground">
                        <div className="flex flex-wrap gap-2">
                          {(group.managerRoles.length > 0 ? group.managerRoles : ['manager']).map(role => (
                            <AgentTypeBadge
                              key={role}
                              type={role === 'manager' ? 'avatar_manager' : 'avatar_service'}
                            />
                          ))}
                        </div>
                      </div>
                    </div>
                    <div className="rounded-xl border border-border/70 bg-muted/10 px-4 py-4">
                      <div className="text-xs text-muted-foreground">
                        {t('agent.manage.managerAgentStatusLabel', '管理 Agent 状态')}
                      </div>
                      <div className="mt-2 flex items-center gap-2">
                        <Bot className="h-4 w-4 text-muted-foreground" />
                        {group.managerAgent ? (
                          <StatusBadge status={AGENT_STATUS_MAP[group.managerAgent.status] || 'neutral'}>
                            {t(`agent.status.${group.managerAgent.status}`)}
                          </StatusBadge>
                        ) : (
                          <span className="text-sm font-medium text-foreground">-</span>
                        )}
                      </div>
                    </div>
                  </div>

                  {managerExtensions.length > 0 && (
                    <div className="space-y-2">
                      <div className="text-sm font-medium text-foreground">
                        {t('agent.extensions.enabled')}
                      </div>
                      <div className="flex flex-wrap gap-2">
                        {managerExtensions.map(name => (
                          <Badge key={name} variant="secondary" className="text-[11px]">
                            {name}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  )}
                </CardContent>
              </Card>

              <Card className="border-border/70">
                <CardHeader>
                  <CardTitle className="text-base">
                    {t('agent.manage.avatarServiceListTitle', '该管理 Agent 下的分身服务 Agent')}
                  </CardTitle>
                </CardHeader>
                  <CardContent>
                  {group.portals.length > 0 ? (
                      <div className="space-y-3">
                        {group.portals.map(portalLink => (
                          <ServiceAvatarRow
                            key={portalLink.portalId}
                            teamId={team.id}
                            portalLink={portalLink}
                            portalDetail={portalDetailsById.get(portalLink.portalId) || null}
                            documentsById={documentsById}
                            canManage={canManage}
                            onEditAvatar={(portal) => {
                              setSelectedAvatar(portal);
                            }}
                            onDeleteAvatar={(portal) => {
                              setDeleteAvatarAndService(false);
                              setDeleteAvatarError('');
                              setAvatarDeleteTarget(portal);
                            }}
                            onDeleteServiceAgent={(agent) => {
                              setSelectedAgent(agent);
                              setDeleteAgentOpen(true);
                            }}
                            onEditServiceAgent={(agent) => {
                              setSelectedAgent(agent);
                              setEditAgentOpen(true);
                            }}
                            onChat={openAgentChat}
                          />
                        ))}
                      </div>
                  ) : (
                    <div className="rounded-xl border border-dashed border-border/70 px-4 py-10 text-sm text-muted-foreground">
                      {t('agent.manage.noAvatarUnderManager', '当前管理 Agent 下还没有分身服务 Agent。')}
                    </div>
                  )}
                </CardContent>
              </Card>
            </>
          ) : (
            <Card className="border-border/70">
              <CardContent className="flex flex-col items-center justify-center gap-4 py-16 text-center">
                <div className="text-lg font-semibold text-foreground">
                  {managerId === UNGROUPED_MANAGER_KEY
                    ? t('agent.manage.ungroupedManagerTitle', '未归类分组')
                    : t('agent.manage.dedicatedManagerNotFound', '未找到对应的数字分身管理 Agent')}
                </div>
                <p className="max-w-xl text-sm text-muted-foreground">
                  {t('agent.manage.dedicatedManagerNotFoundHint', '当前分组可能已被删除、重新绑定，或尚未生成新的分身服务 Agent。')}
                </p>
                <Button variant="outline" onClick={() => navigate(`/teams/${teamId}?section=agent`)}>
                  {t('agent.manage.backToAgentManage', '返回 Agent 管理')}
                </Button>
              </CardContent>
            </Card>
          )}
        </div>

        <CreateInviteDialog
          open={inviteDialogOpen}
          onOpenChange={setInviteDialogOpen}
          teamId={team.id}
          onCreated={() => void 0}
        />

        <EditAgentDialog
          agent={selectedAgent}
          open={editAgentOpen}
          onOpenChange={setEditAgentOpen}
          onUpdated={loadData}
        />
        <DeleteAgentDialog
          agent={selectedAgent}
          open={deleteAgentOpen}
          onOpenChange={setDeleteAgentOpen}
          onDeleted={loadData}
        />

        <EditAvatarDialog
          teamId={team.id}
          portal={selectedAvatar}
          serviceAgent={
            selectedAvatar
              ? group?.portals.find(item => item.portalId === selectedAvatar.id)?.serviceAgent || null
              : null
          }
          documentsById={documentsById}
          open={Boolean(selectedAvatar)}
          onOpenChange={(open) => {
            if (!open) {
              setSelectedAvatar(null);
            }
          }}
          onSaved={loadData}
        />

        <ConfirmDialog
          open={Boolean(avatarDeleteTarget)}
          onOpenChange={(open) => {
            if (!open) {
              setDeleteAvatarAndService(false);
              setDeleteAvatarError('');
              setAvatarDeleteTarget(null);
            }
          }}
          title={t('agent.manage.deleteAvatarTitle', '删除分身')}
          description={t(
            'agent.manage.deleteAvatarDescription',
            'This deletes the public entry and permission configuration of the current digital avatar. You can also choose to clean up the dedicated service agent used only by this avatar.'
          )}
          confirmText={t('common.delete')}
          variant="destructive"
          onConfirm={handleDeleteAvatar}
          loading={deletingAvatar}
        >
          <div className="space-y-3">
            {avatarDeleteTarget ? (
              <div className="rounded-lg border border-border/60 bg-muted/10 px-3 py-3">
                <div className="text-xs font-medium text-foreground">
                  {t('agent.manage.deleteAvatarTargetLabel', '删除对象')}
                </div>
                <div className="mt-2 space-y-1 text-sm text-muted-foreground">
                  <div>
                    <span className="font-medium text-foreground">
                      {t('agent.manage.deleteAvatarTargetName', '分身名称')}
                    </span>
                    <span className="ml-2">{avatarDeleteTarget.portalName}</span>
                  </div>
                  <div>
                    <span className="font-medium text-foreground">
                      {t('agent.manage.deleteAvatarTargetSlug', '访问地址')}
                    </span>
                    <span className="ml-2">{avatarDeleteTarget.portalSlug ? `/${avatarDeleteTarget.portalSlug}` : '-'}</span>
                  </div>
                  <div>
                    <span className="font-medium text-foreground">
                      {t('agent.manage.deleteAvatarTargetServiceAgent', '底层服务 Agent')}
                    </span>
                    <span className="ml-2">{avatarDeleteTarget.serviceAgent?.name || '-'}</span>
                  </div>
                </div>
              </div>
            ) : null}

            <div className="rounded-lg border border-border/60 bg-background px-3 py-3">
              <div className="text-xs font-medium text-foreground">
                {t('agent.manage.deleteAvatarImpactLabel', '处理结果')}
              </div>
              <ul className="mt-2 space-y-1 text-xs text-muted-foreground">
                <li>{t('agent.manage.deleteAvatarImpactPortal', '会删除分身入口、公开地址和访客权限配置。')}</li>
                <li>{t('agent.manage.deleteAvatarImpactManager', '不会影响管理 Agent。')}</li>
                <li>
                  {deleteAvatarAndService
                    ? t('agent.manage.deleteAvatarImpactServiceIncluded', '会同时清理这个分身专用的底层服务 Agent。')
                    : t('agent.manage.deleteAvatarImpactServiceExcluded', '默认保留底层服务 Agent，避免误删执行能力。')}
                </li>
              </ul>
            </div>

            {canCleanupDedicatedService && avatarDeleteTarget?.serviceAgent ? (
              <label className="flex items-start gap-3 rounded-lg border border-border/60 bg-muted/10 px-3 py-3 text-sm">
                <input
                  type="checkbox"
                  className="mt-0.5 h-4 w-4"
                  checked={deleteAvatarAndService}
                  onChange={(event) => setDeleteAvatarAndService(event.target.checked)}
                />
                <span className="space-y-1">
                  <span className="block font-medium text-foreground">
                    {t('agent.manage.deleteAvatarCleanupServiceLabel', '同时清理专用服务 Agent')}
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    {t(
                      'agent.manage.deleteAvatarCleanupServiceHint',
                      'Deletion only runs when this service agent is used exclusively by the current avatar, to avoid deleting shared execution capability by mistake.'
                    )}
                  </span>
                </span>
              </label>
            ) : avatarDeleteTarget?.serviceAgent ? (
              <div className="rounded-lg border border-border/60 bg-muted/10 px-3 py-3 text-xs text-muted-foreground">
                {avatarDeleteServiceUsageCount > 1
                  ? t(
                    'agent.manage.deleteAvatarCleanupBlockedShared',
                    'The underlying service agent is still reused by other avatars. Right now only the avatar entry can be deleted; the service agent cannot be cleaned up automatically.'
                  )
                  : t(
                    'agent.manage.deleteAvatarCleanupBlockedGeneric',
                    'The underlying service agent is not a dedicated avatar agent. Right now only the avatar entry will be deleted, and the shared agent will not be affected.'
                  )}
              </div>
            ) : null}

            {deleteAvatarError ? (
              <div className="text-sm text-destructive">{deleteAvatarError}</div>
            ) : null}
          </div>
        </ConfirmDialog>
      </AppShell>
    </TeamProvider>
  );
}

