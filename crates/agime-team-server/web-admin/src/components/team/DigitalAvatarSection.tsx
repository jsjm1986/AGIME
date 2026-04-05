import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  Check,
  CircleSlash,
  Clock3,
  ExternalLink,
  Loader2,
  Plus,
  RefreshCw,
  ShieldAlert,
  X,
  UserRound,
  Users,
} from 'lucide-react';
import { AgentTypeBadge, resolveAgentVisualType } from '../agent/AgentTypeBadge';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../ui/dialog';
import {
  Select as UiSelect,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { Textarea } from '../ui/textarea';
import {
  avatarPortalApi,
  type AvatarGovernanceEventPayload,
  type AvatarGovernanceQueueItemPayload,
  type AvatarInstanceProjection,
  type PortalDetail,
  type PortalDocumentAccessMode,
  type PortalSummary,
} from '../../api/avatarPortal';
import { chatApi, type ChatSessionEvent } from '../../api/chat';
import { BUILTIN_EXTENSIONS, agentApi, type TeamAgent } from '../../api/agent';
import {
  documentApi,
  type DocumentSummary,
} from '../../api/documents';
import { ChatConversation, type ChatRuntimeEvent } from '../chat/ChatConversation';
import type { ChatInputQuickActionGroup } from '../chat/ChatInput';
import type { ChatInputComposeRequest } from '../chat/ChatInput';
import { DocumentPicker } from '../documents/DocumentPicker';
import { useToast } from '../../contexts/ToastContext';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { useMediaQuery } from '../../hooks/useMediaQuery';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { ManagementRail } from '../mobile/ManagementRail';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';
import { CreateAvatarDialog } from './digital-avatar/CreateAvatarDialog';
import { CreateManagerAgentDialog } from './digital-avatar/CreateManagerAgentDialog';
import { DigitalAvatarGuide } from './digital-avatar/DigitalAvatarGuide';
import { formatDateTime, formatRelativeTime } from '../../utils/format';
import { copyText } from '../../utils/clipboard';
import {
  createEmptyGovernanceState,
  makeId,
  mergeGovernanceAutomationConfig,
  mergeGovernanceSettings,
  readGovernanceAutomationConfig,
  readGovernanceState,
  DEFAULT_AUTOMATION_CONFIG,
  type AgentGapProposal,
  type AvatarGovernanceAutomationConfig,
  type AvatarGovernanceState,
  type CapabilityGapRequest,
  type DecisionMode,
  type OptimizationStatus,
  type OptimizationTicket,
  type ProposalStatus,
  type RuntimeLogEntry,
  type RuntimeLogStatus,
} from './digital-avatar/governance';
import { detectAvatarType } from './digital-avatar/avatarType';
import { AvatarTypeBadge } from './digital-avatar/AvatarTypeBadge';
import {
  getDigitalAvatarManagerId,
  isDigitalAvatarPortal,
  splitGeneralAndDedicatedAgents,
} from './agentIsolation';
import {
  buildAvatarPublicNarrativePayload,
  joinNarrativeUseCases,
  readAvatarPublicNarrative,
  splitNarrativeUseCases,
} from '../../lib/avatarPublicNarrative';

interface DigitalAvatarSectionProps {
  teamId: string;
  canManage: boolean;
}

type AvatarFilter = 'all' | 'external' | 'internal';
type WorkspaceTab = 'workspace' | 'guide';
type InspectorTab = 'overview' | 'permissions' | 'publish';
type RuntimeLogFilter = 'pending' | 'all';
type GovernanceKindFilter = 'all' | 'capability' | 'proposal' | 'ticket' | 'runtime';
type GovernanceRiskFilter = 'all' | 'low' | 'medium' | 'high';
type PersistedEventFilter = 'all' | 'error' | 'tool' | 'thinking' | 'status';
type PublishViewMode = 'visitor' | 'preview' | 'test';
type RuntimeSuggestion = RuntimeLogEntry;
type PersistedEventLoadMode = 'latest' | 'older' | 'incremental';
type MobileWorkspacePanel = 'avatar-switcher' | 'console' | 'guide' | null;
type RuntimeExtensionOption = {
  id: string;
  label: string;
  description?: string;
};
const MANAGER_COMPOSE_STORAGE_PREFIX = 'digital_avatar_manager_compose:v1:';
const MANAGER_FOCUS_STORAGE_PREFIX = 'digital_avatar_focus:v1:';
const WORKSPACE_CHROME_STORAGE_PREFIX = 'digital_avatar_workspace_chrome:v1:';
const RUNTIME_EXTENSION_NAME_ALIAS: Record<string, string> = {
  auto_visualiser: 'autovisualiser',
  computer_controller: 'computercontroller',
};
const PRIMARY_INSPECTOR_TABS: InspectorTab[] = ['overview', 'permissions', 'publish'];
const WORKSPACE_SHELL_CLASS = 'bg-transparent px-0 py-0';
const WORKSPACE_PANEL_CLASS =
  'rounded-[24px] border border-transparent bg-[hsl(var(--ui-surface-panel-strong))/0.985] text-foreground shadow-[0_10px_28px_hsl(var(--ui-shadow))/0.035] dark:bg-[hsl(var(--ui-surface-panel-strong))/0.985] dark:shadow-[0_18px_40px_hsl(var(--ui-shadow))/0.26]';
const CONTROL_DECK_CLASS = 'relative px-3 py-1.5';
const AVATAR_NAV_PANEL_CLASS =
  'rounded-[24px] border border-transparent bg-[hsl(var(--ui-surface-panel-muted))/0.96] text-foreground shadow-[0_8px_24px_hsl(var(--ui-shadow))/0.028] dark:bg-[hsl(var(--ui-surface-panel-muted))/0.94] dark:shadow-[0_16px_38px_hsl(var(--ui-shadow))/0.24]';
const INSPECTOR_PANEL_CLASS =
  'rounded-[24px] border border-transparent bg-[hsl(var(--ui-surface-panel-strong))/0.985] text-foreground shadow-[0_10px_28px_hsl(var(--ui-shadow))/0.03] dark:bg-[hsl(var(--ui-surface-panel-strong))/0.97] dark:shadow-[0_18px_40px_hsl(var(--ui-shadow))/0.28]';
const INSPECTOR_SECTION_CLASS =
  'rounded-[22px] border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel-muted))/0.92] px-4 py-4 shadow-[0_10px_24px_hsl(var(--ui-shadow))/0.04] dark:bg-[hsl(var(--ui-surface-panel-muted))/0.88] lg:rounded-none lg:border-x-0 lg:border-b-0 lg:border-t lg:border-[hsl(var(--ui-line-soft))/0.44] lg:bg-transparent lg:px-0 lg:py-5 lg:shadow-none first:lg:border-t-0 first:lg:pt-0';
const INSPECTOR_ACTION_LINK_CLASS =
  'inline-flex appearance-none items-center gap-1 border-0 bg-transparent p-0 text-[12px] font-medium leading-none whitespace-nowrap text-muted-foreground shadow-none transition-colors hover:text-foreground disabled:pointer-events-none disabled:text-muted-foreground/70';
const AVATAR_PRIMARY_BUTTON_CLASS =
  'border-primary bg-primary text-primary-foreground hover:border-primary/90 hover:bg-primary/92';
const BARE_BUTTON_CLASS = 'appearance-none border-0 bg-transparent p-0 shadow-none outline-none';
const CONTROL_ROOM_CHROME_CLASS = 'px-0 py-0';
const CONTROL_ROOM_TOOLBAR_BUTTON_CLASS =
  'h-auto rounded-none border-0 bg-transparent px-0 py-0 text-[12px] font-medium text-muted-foreground shadow-none transition-colors hover:text-foreground';

function InspectorSection({
  title,
  description,
  action,
  children,
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className={INSPECTOR_SECTION_CLASS}>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0">
          <h3 className="text-[11px] font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            {title}
          </h3>
          {description ? (
            <div className="mt-1.5 max-w-[34ch] text-[12px] leading-6 text-muted-foreground">
              {description}
            </div>
          ) : null}
        </div>
        {action ? <div className="shrink-0 pt-0.5">{action}</div> : null}
      </div>
      <div className="mt-3">{children}</div>
    </section>
  );
}

function InspectorField({
  label,
  value,
  hint,
  alignTop = false,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  alignTop?: boolean;
}) {
  return (
    <div className="py-3 first:pt-0">
      <div className="text-[10px] font-medium tracking-[0.12em] text-muted-foreground uppercase">
        {label}
      </div>
      <div className={`min-w-0 ${alignTop ? 'mt-1.5' : 'mt-1'}`}>
        <div className="min-w-0 break-words text-[13px] font-medium leading-6 text-foreground">
          {value}
        </div>
        {hint ? <div className="mt-1.5 break-words text-[12px] leading-6 text-muted-foreground">{hint}</div> : null}
      </div>
    </div>
  );
}

function isAvatar(summary: PortalSummary): boolean {
  return isDigitalAvatarPortal(summary);
}

function normalizeAvatarStatus(summary: PortalSummary | PortalDetail | null | undefined): string {
  return (summary?.status || '').trim().toLowerCase();
}

function getAvatarProjectionPendingCount(projection: AvatarInstanceProjection | null | undefined): number {
  if (!projection) return 0;
  return (
    (projection.governanceCounts.pendingCapabilityRequests || 0) +
    (projection.governanceCounts.pendingGapProposals || 0) +
    (projection.governanceCounts.pendingOptimizationTickets || 0) +
    (projection.governanceCounts.pendingRuntimeLogs || 0)
  );
}

function buildGovernanceCounts(state: AvatarGovernanceState) {
  return {
    pendingCapabilityRequests: state.capabilityRequests.filter((item) => item.status === 'pending').length,
    pendingGapProposals: state.gapProposals.filter((item) => item.status === 'pending_approval').length,
    pendingOptimizationTickets: state.optimizationTickets.filter((item) => item.status === 'pending').length,
    pendingRuntimeLogs: state.runtimeLogs.filter((item) => item.status === 'pending').length,
  };
}

function resolveManagerGroupCandidates(agents: TeamAgent[], avatars: PortalSummary[]): TeamAgent[] {
  const { managerDedicatedAgents } = splitGeneralAndDedicatedAgents(agents, avatars);
  return managerDedicatedAgents;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function getAgentDisplayName(agent: TeamAgent | null | undefined, fallback = 'N/A'): string {
  if (!agent?.name) return fallback;

  const visualType = resolveAgentVisualType(agent);
  const suffixes = [
    '管理Agent',
    '管理 Agent',
    '分身管理Agent',
    '分身管理 Agent',
    '服务Agent',
    '服务 Agent',
    '分身服务Agent',
    '分身服务 Agent',
  ];

  let normalized = agent.name.trim();
  for (const suffix of suffixes) {
    normalized = normalized.replace(
      new RegExp(`(?:\\s*[-－—]\\s*${escapeRegExp(suffix)})+$`, 'giu'),
      '',
    ).trim();
  }

  if (visualType === 'avatar_manager') {
    normalized = normalized
      .replace(/(?:管理\s*Agent)(?:\s*[-－—]?\s*管理\s*Agent)+$/giu, '管理Agent')
      .replace(/(?:分身管理\s*Agent)(?:\s*[-－—]?\s*分身管理\s*Agent)+$/giu, '分身管理Agent')
      .trim();
  }

  if (visualType === 'avatar_service') {
    normalized = normalized
      .replace(/(?:服务\s*Agent)(?:\s*[-－—]?\s*服务\s*Agent)+$/giu, '服务Agent')
      .replace(/(?:分身服务\s*Agent)(?:\s*[-－—]?\s*分身服务\s*Agent)+$/giu, '分身服务Agent')
      .trim();
  }

  return normalized || agent.name || fallback;
}

function getAgentName(agents: TeamAgent[], id: string | null | undefined, fallback = 'N/A'): string {
  if (!id) return fallback;
  const agent = agents.find((item) => item.id === id);
  return agent ? getAgentDisplayName(agent, fallback) : id;
}

function getEnabledExtensionEntries(agent: TeamAgent | null | undefined): Array<{ id: string; name: string }> {
  if (!agent) return [];
  return (agent.enabled_extensions || [])
    .filter((entry) => entry.enabled)
    .map((entry) => {
      const builtin = BUILTIN_EXTENSIONS.find((item) => item.id === entry.extension);
      return {
        id: entry.extension,
        name: builtin?.name || entry.extension,
      };
    });
}

function getAssignedSkillEntries(agent: TeamAgent | null | undefined): Array<{ id: string; name: string }> {
  if (!agent) return [];
  return (agent.assigned_skills || [])
    .filter((entry) => entry.enabled)
    .map((entry) => ({
      id: entry.skill_id,
      name: entry.name || entry.skill_id,
    }));
}

function getRuntimeExtensionId(extensionId: string) {
  return RUNTIME_EXTENSION_NAME_ALIAS[extensionId] || extensionId;
}

function resolveExtensionLabel(id: string) {
  const builtin = BUILTIN_EXTENSIONS.find(
    (item) => item.id === id || getRuntimeExtensionId(item.id) === id,
  );
  return builtin?.name || id;
}

function formatPortalOutputForm(
  value: PortalSummary['outputForm'] | PortalDetail['outputForm'] | undefined,
  t: (key: string, fallback: string) => string,
) {
  switch (value) {
    case 'website':
      return t('digitalAvatar.workspace.outputFormWebsite', '完整页面');
    case 'widget':
      return t('digitalAvatar.workspace.outputFormWidget', '嵌入挂件');
    case 'agent_only':
      return t('digitalAvatar.workspace.outputFormAgentOnly', 'Agent Only');
    default:
      return t('digitalAvatar.labels.unset', '未设置');
  }
}

function formatPublicExposure(
  value: string | undefined,
  t: (key: string, fallback: string) => string,
) {
  switch (value) {
    case 'public_page':
      return t('digitalAvatar.workspace.exposurePublicPage', '正式访客页');
    case 'preview_only':
      return t('digitalAvatar.workspace.exposurePreviewOnly', '仅管理预览');
    default:
      return t('digitalAvatar.labels.unset', '未设置');
  }
}

function getRuntimeExtensionOptions(agent: TeamAgent | null | undefined): RuntimeExtensionOption[] {
  if (!agent) return [];
  const seen = new Set<string>();
  return (agent.enabled_extensions || [])
    .filter((entry) => entry.enabled)
    .map((entry) => {
      const builtin = BUILTIN_EXTENSIONS.find((item) => item.id === entry.extension);
      return {
        id: getRuntimeExtensionId(entry.extension),
        label: builtin?.name || entry.extension,
        description: builtin?.description,
      };
    })
    .filter((item) => {
      if (seen.has(item.id)) return false;
      seen.add(item.id);
      return true;
    });
}

function toggleSelection<T>(items: T[], value: T): T[] {
  return items.includes(value) ? items.filter((item) => item !== value) : [...items, value];
}

function normalizeStringSelection(items: string[]): string[] {
  return Array.from(new Set(items)).sort((a, b) => a.localeCompare(b));
}

function sameStringSelection(left: string[], right: string[]): boolean {
  const a = normalizeStringSelection(left);
  const b = normalizeStringSelection(right);
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

function renderCapabilityChipList(
  items: Array<{ id: string; name: string }>,
  emptyLabel: string,
) {
  if (items.length === 0) {
    return <span className="text-[12px] text-muted-foreground">{emptyLabel}</span>;
  }

  return (
    <div className="mt-1 flex flex-wrap gap-2">
      {items.map((item) => (
        <Badge
          key={`${item.id}-${item.name}`}
          variant="secondary"
          className="max-w-full whitespace-normal break-words px-2 py-1 text-left text-[11px] leading-5"
        >
          {item.name}
        </Badge>
      ))}
    </div>
  );
}

function renderRemovableCapabilityChipList(
  items: Array<{ id: string; name: string }>,
  emptyLabel: string,
  onRemove?: (id: string) => void,
) {
  if (items.length === 0) {
    return <span className="text-[12px] text-muted-foreground">{emptyLabel}</span>;
  }

  return (
    <div className="mt-1 flex flex-wrap gap-2">
      {items.map((item) =>
        onRemove ? (
          <button
            key={`${item.id}-${item.name}`}
            type="button"
            className="inline-flex max-w-full items-start gap-1 rounded-full border border-border/60 bg-secondary px-2.5 py-1 text-left text-[11px] font-medium leading-5 text-secondary-foreground transition-colors hover:border-destructive/30 hover:bg-destructive/10 hover:text-destructive"
            onClick={() => onRemove(item.id)}
          >
            <span className="break-words whitespace-normal">{item.name}</span>
            <X className="mt-0.5 h-3 w-3 shrink-0" />
          </button>
        ) : (
          <Badge
            key={`${item.id}-${item.name}`}
            variant="secondary"
            className="max-w-full whitespace-normal break-words px-2 py-1 text-left text-[11px] leading-5"
          >
            {item.name}
          </Badge>
        ),
      )}
    </div>
  );
}

function formatDocumentAccessMode(
  mode: PortalDetail['documentAccessMode'] | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (mode) {
    case 'read_only':
      return t('digitalAvatar.documentAccess.readOnly', '只读');
    case 'co_edit_draft':
      return t('digitalAvatar.documentAccess.coEditDraft', '协作草稿');
    case 'controlled_write':
      return t('digitalAvatar.documentAccess.controlledWrite', '受控写入');
    default:
      return t('digitalAvatar.labels.unset');
  }
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

function toIsoNow(): string {
  return new Date().toISOString();
}

function toRisk(score: number): 'low' | 'medium' | 'high' {
  if (score >= 0.75) return 'high';
  if (score >= 0.35) return 'medium';
  return 'low';
}

function extractRiskFromTexts(texts: string[]): GovernanceRiskFilter {
  const joined = texts.join(' ').toLowerCase();
  if (!joined) return 'low';
  if (
    joined.includes('high')
    || joined.includes('高风险')
    || joined.includes('critical')
    || joined.includes('严重')
  ) {
    return 'high';
  }
  if (
    joined.includes('medium')
    || joined.includes('中风险')
    || joined.includes('moderate')
  ) {
    return 'medium';
  }
  return 'low';
}

function buildPermissionPreview(
  mode: PortalDetail['documentAccessMode'] | undefined,
  t: (key: string, fallback: string) => string,
): string[] {
  return [
    mode === 'read_only'
      ? t('digitalAvatar.permission.readOnly', '只允许读取/检索绑定文档')
      : t('digitalAvatar.permission.readWrite', '允许读取/检索绑定文档，并可创建或更新内容'),
    mode === 'co_edit_draft'
      ? t('digitalAvatar.permission.draftOnly', '更新仅限协作草稿范围')
      : mode === 'read_only'
      ? t('digitalAvatar.permission.noUpdate', '当前不允许更新文档')
      : t('digitalAvatar.permission.controlledWrite', '写入会经过受控写入策略与校验'),
    t('digitalAvatar.permission.boundary', '所有行为仍受绑定文档范围与允许能力约束'),
  ];
}

function avatarStatusBadgeClass(status: string): string {
  switch (status) {
    case 'published':
      return 'border border-[hsl(var(--status-success-text))/0.16] bg-status-success-bg text-status-success-text';
    case 'draft':
      return 'border border-[hsl(var(--status-warning-text))/0.18] bg-status-warning-bg text-status-warning-text';
    case 'disabled':
    case 'archived':
      return 'border border-border/70 bg-muted/30 text-muted-foreground';
    default:
      return 'border border-border/70 bg-muted/30 text-muted-foreground';
  }
}

function avatarStatusLabel(
  status: string,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (status) {
    case 'published':
      return t('digitalAvatar.status.published', '已发布');
    case 'draft':
      return t('digitalAvatar.status.draft', '草稿');
    case 'disabled':
      return t('digitalAvatar.status.disabled', '已停用（兼容）');
    case 'archived':
      return t('digitalAvatar.status.archived', '已归档');
    default:
      return t('digitalAvatar.labels.unset', '未配置');
  }
}

function toDecisionStatus(decision: DecisionMode): CapabilityGapRequest['status'] {
  if (decision === 'approve_direct' || decision === 'approve_sandbox') return 'approved';
  if (decision === 'require_human_confirm') return 'needs_human';
  return 'rejected';
}

function toProposalStatusLabel(status: ProposalStatus): string {
  switch (status) {
    case 'pending_approval':
      return 'pending_approval';
    case 'approved':
      return 'approved';
    case 'rejected':
      return 'rejected';
    case 'pilot':
      return 'pilot';
    case 'active':
      return 'active';
    default:
      return 'draft';
  }
}

function toOptimizationStatusLabel(status: OptimizationStatus): string {
  switch (status) {
    case 'approved':
    case 'rejected':
    case 'experimenting':
    case 'deployed':
    case 'rolled_back':
      return status;
    default:
      return 'pending';
  }
}

function getGovernanceStatusLabel(
  t: ReturnType<typeof useTranslation>['t'],
  status: string,
): string {
  switch (status) {
    case 'pending':
      return t('digitalAvatar.governance.status.pending', '待决策');
    case 'approved':
      return t('digitalAvatar.governance.status.approved', '已通过');
    case 'needs_human':
      return t('digitalAvatar.governance.status.needs_human', '需人工确认');
    case 'rejected':
      return t('digitalAvatar.governance.status.rejected', '已拒绝');
    default:
      return status;
  }
}

function getProposalStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: ProposalStatus | string,
): string {
  const normalized = toProposalStatusLabel(status as ProposalStatus);
  switch (normalized) {
    case 'draft':
      return t('digitalAvatar.governance.proposalStatus.draft', '草稿');
    case 'pending_approval':
      return t('digitalAvatar.governance.proposalStatus.pending_approval', '待审批');
    case 'approved':
      return t('digitalAvatar.governance.proposalStatus.approved', '已通过');
    case 'rejected':
      return t('digitalAvatar.governance.proposalStatus.rejected', '已拒绝');
    case 'pilot':
      return t('digitalAvatar.governance.proposalStatus.pilot', '试运行');
    case 'active':
      return t('digitalAvatar.governance.proposalStatus.active', '生效中');
    default:
      return status;
  }
}

function getOptimizationStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: OptimizationStatus | string,
): string {
  const normalized = toOptimizationStatusLabel(status as OptimizationStatus);
  switch (normalized) {
    case 'pending':
      return t('digitalAvatar.governance.ticketStatus.pending', '待审批');
    case 'approved':
      return t('digitalAvatar.governance.ticketStatus.approved', '已通过');
    case 'rejected':
      return t('digitalAvatar.governance.ticketStatus.rejected', '已拒绝');
    case 'experimenting':
      return t('digitalAvatar.governance.ticketStatus.experimenting', '实验中');
    case 'deployed':
      return t('digitalAvatar.governance.ticketStatus.deployed', '已部署');
    case 'rolled_back':
      return t('digitalAvatar.governance.ticketStatus.rolled_back', '已回滚');
    default:
      return status;
  }
}

function getRuntimeStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  status: RuntimeLogStatus | string,
): string {
  switch (status) {
    case 'pending':
      return t('digitalAvatar.governance.runtimeStatus.pending', '待处理');
    case 'ticketed':
      return t('digitalAvatar.governance.runtimeStatus.ticketed', '已转工单');
    case 'requested':
      return t('digitalAvatar.governance.runtimeStatus.requested', '已转提权');
    case 'dismissed':
      return t('digitalAvatar.governance.runtimeStatus.dismissed', '已忽略');
    default:
      return status;
  }
}

function getGovernanceItemStatusText(
  t: ReturnType<typeof useTranslation>['t'],
  item: { kind?: 'capability' | 'proposal' | 'ticket'; rowType?: 'runtime' | 'capability' | 'proposal' | 'ticket'; status: string },
): string {
  if (item.rowType === 'runtime') {
    return getRuntimeStatusText(t, item.status);
  }
  if (item.kind === 'proposal' || item.rowType === 'proposal') {
    return getProposalStatusText(t, item.status);
  }
  if (item.kind === 'ticket' || item.rowType === 'ticket') {
    return getOptimizationStatusText(t, item.status);
  }
  return getGovernanceStatusLabel(t, item.status);
}

interface RuntimeSuggestionText {
  unknownTool: string;
  toolFailureTitle: (tool: string) => string;
  toolFailureEvidenceFallback: string;
  toolFailureProposal: (tool: string) => string;
  toolFailureGain: string;
  sessionFailedTitle: string;
  sessionFailedProposal: string;
  sessionFailedGain: string;
}

const MAX_RUNTIME_LOGS = 80;
const PERSISTED_EVENTS_PAGE_SIZE = 200;

interface GovernanceExecutionBinding {
  id: string;
  entityType: 'capability' | 'proposal' | 'ticket';
  targetId: string;
  targetStatus: string;
}

interface GovernanceActionReceipt {
  actionId: string;
  outcome: 'success' | 'partial' | 'failed';
  summary?: string;
  reason?: string;
}

function persistedEventKey(event: ChatSessionEvent): string {
  return `${event.run_id || 'no_run'}:${event.event_id}:${event.created_at}`;
}

function mergePersistedEvents(
  base: ChatSessionEvent[],
  incoming: ChatSessionEvent[],
  mode: PersistedEventLoadMode,
): ChatSessionEvent[] {
  if (incoming.length === 0) return base;
  const merged = mode === 'older' ? [...incoming, ...base] : [...base, ...incoming];
  const dedup = new Map<string, ChatSessionEvent>();
  for (const item of merged) {
    dedup.set(persistedEventKey(item), item);
  }
  return Array.from(dedup.values()).sort((a, b) => {
    if (a.event_id !== b.event_id) return a.event_id - b.event_id;
    return Date.parse(a.created_at) - Date.parse(b.created_at);
  });
}

function isRuntimeDoneFailure(detail: Record<string, unknown> | undefined): boolean {
  const error = typeof detail?.error === 'string' ? detail.error.trim() : '';
  if (error) return true;
  const status = typeof detail?.status === 'string' ? detail.status.trim().toLowerCase() : '';
  if (!status) return false;
  return status.includes('fail') || status.includes('error') || status.includes('timeout') || status.includes('cancel');
}

function parseGovernanceActionReceipts(text: string): GovernanceActionReceipt[] {
  if (!text.trim()) return [];
  const receipts: GovernanceActionReceipt[] = [];
  const tagPattern = /<governance_action_result>([\s\S]*?)<\/governance_action_result>/gi;
  let match: RegExpExecArray | null;
  while ((match = tagPattern.exec(text)) !== null) {
    const raw = (match[1] || '').trim();
    if (!raw) continue;
    try {
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      const actionId = String(parsed.action_id || parsed.actionId || '').trim();
      const outcomeRaw = String(parsed.outcome || '').trim().toLowerCase();
      const outcome = outcomeRaw === 'success' || outcomeRaw === 'partial' || outcomeRaw === 'failed'
        ? (outcomeRaw as GovernanceActionReceipt['outcome'])
        : null;
      if (!actionId || !outcome) continue;
      receipts.push({
        actionId,
        outcome,
        summary: typeof parsed.summary === 'string' ? parsed.summary.trim() : undefined,
        reason: typeof parsed.reason === 'string' ? parsed.reason.trim() : undefined,
      });
    } catch {
      // Ignore malformed result block.
    }
  }
  return receipts;
}

function summarizeRuntimeFailure(
  event: ChatRuntimeEvent,
  text: RuntimeSuggestionText,
): RuntimeSuggestion | null {
  const detail = event.detail || {};
  if (event.kind === 'toolresult') {
    const success = detail.success;
    if (success !== false) return null;
    const toolName = typeof detail.toolName === 'string' && detail.toolName.trim()
      ? detail.toolName.trim()
      : text.unknownTool;
    const preview = typeof detail.preview === 'string' ? detail.preview.trim() : '';
    const evidence = preview || event.text || text.toolFailureEvidenceFallback;
    const risk = toRisk(preview.length > 120 ? 0.65 : 0.4);
    return {
      id: makeId('runtime'),
      title: text.toolFailureTitle(toolName),
      evidence,
      proposal: text.toolFailureProposal(toolName),
      expectedGain: text.toolFailureGain,
      risk,
      problemType: 'tool',
      createdAt: toIsoNow(),
      status: 'pending',
    };
  }
  if (event.kind === 'done' && typeof detail.error === 'string' && detail.error.trim()) {
    const error = detail.error.trim();
    return {
      id: makeId('runtime'),
      title: text.sessionFailedTitle,
      evidence: error,
      proposal: text.sessionFailedProposal,
      expectedGain: text.sessionFailedGain,
      risk: 'medium',
      problemType: 'policy',
      createdAt: toIsoNow(),
      status: 'pending',
    };
  }
  return null;
}

function badgeClass(status: string): string {
  if (status === 'approved' || status === 'deployed' || status === 'active') {
    return 'bg-status-success/15 text-status-success-text border-status-success/40';
  }
  if (status === 'rejected' || status === 'deny' || status === 'rolled_back') {
    return 'bg-status-error/15 text-status-error-text border-status-error/40';
  }
  if (status === 'pending' || status === 'pending_approval' || status === 'needs_human') {
    return 'bg-status-warning/15 text-status-warning-text border-status-warning/40';
  }
  return 'bg-muted text-muted-foreground border-border/60';
}

function runtimeStatusClass(status: RuntimeLogStatus): string {
  if (status === 'pending') {
    return 'bg-status-warning/15 text-status-warning-text border-status-warning/40';
  }
  if (status === 'ticketed' || status === 'requested') {
    return 'bg-status-success/15 text-status-success-text border-status-success/40';
  }
  return 'bg-muted text-muted-foreground border-border/60';
}

function isOpenGovernanceStatus(status: string): boolean {
  return [
    'pending',
    'pending_approval',
    'needs_human',
    'approved',
    'pilot',
    'experimenting',
    'ticketed',
    'requested',
    'active',
  ].includes(status);
}

function isHumanReviewQueueItem(
  kind: 'capability' | 'proposal' | 'ticket',
  status: string,
): boolean {
  if (kind === 'proposal' || kind === 'ticket') return true;
  return status === 'needs_human';
}

function eventSeverity(event: ChatSessionEvent): 'error' | 'warn' | 'info' {
  if (event.event_type === 'done') {
    const payload = event.payload || {};
    const errorText = typeof payload.error === 'string' ? payload.error.trim() : '';
    const status = typeof payload.status === 'string' ? payload.status.toLowerCase() : '';
    if (errorText || status === 'failed' || status === 'error') return 'error';
    return 'info';
  }
  if (event.event_type === 'toolresult') {
    const success = (event.payload || {}).success;
    return success === false ? 'error' : 'info';
  }
  if (event.event_type === 'status') {
    const raw = String((event.payload || {}).status || '').toLowerCase();
    if (raw.includes('error') || raw.includes('failed') || raw.includes('timeout')) return 'warn';
    return 'info';
  }
  return 'info';
}

function eventTypeBadge(eventType: string): string {
  if (eventType === 'toolcall' || eventType === 'toolresult') return 'tool';
  if (eventType === 'thinking' || eventType === 'turn' || eventType === 'compaction') return 'thinking';
  if (eventType === 'status' || eventType === 'done' || eventType === 'workspace_changed') return 'status';
  return eventType;
}

type PersistedEventDisplayRow = ChatSessionEvent & {
  displayKey: string;
  mergedCount?: number;
};

function canMergePersistedEvent(event: ChatSessionEvent): boolean {
  return event.event_type === 'text' || event.event_type === 'thinking';
}

function mergePersistedEventsForDisplay(events: ChatSessionEvent[]): PersistedEventDisplayRow[] {
  const rows: PersistedEventDisplayRow[] = [];
  for (const event of events) {
    const normalizedEvent: PersistedEventDisplayRow = {
      ...event,
      payload: { ...(event.payload || {}) },
      displayKey: `${event.run_id || 'no_run'}:${event.event_id}:${event.created_at}`,
    };
    const previous = rows[rows.length - 1];
    if (
      previous
      && canMergePersistedEvent(previous)
      && canMergePersistedEvent(normalizedEvent)
      && previous.event_type === normalizedEvent.event_type
      && (previous.run_id || '') === (normalizedEvent.run_id || '')
    ) {
      const previousContent = String(previous.payload?.content || '');
      const nextContent = String(normalizedEvent.payload?.content || '');
      previous.payload = {
        ...previous.payload,
        content: `${previousContent}${nextContent}`,
      };
      previous.created_at = normalizedEvent.created_at;
      previous.mergedCount = (previous.mergedCount || 1) + 1;
      previous.displayKey = `${previous.displayKey}:${normalizedEvent.event_id}`;
      continue;
    }
    rows.push(normalizedEvent);
  }
  return rows;
}

function eventSummary(event: ChatSessionEvent): string {
  const payload = event.payload || {};
  if (event.event_type === 'text' || event.event_type === 'thinking') {
    return String(payload.content || '')
      .replace(/\r\n/g, '\n')
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }
  if (event.event_type === 'status') {
    return String(payload.status || '').trim();
  }
  if (event.event_type === 'toolcall') {
    const name = String(payload.name || '').trim();
    const id = String(payload.id || '').trim();
    return name ? `${name}${id ? ` (${id})` : ''}` : id;
  }
  if (event.event_type === 'toolresult') {
    const name = String(payload.name || '').trim();
    const id = String(payload.id || '').trim();
    const success = payload.success === false ? 'failed' : 'ok';
    const content = String(payload.content || '').trim();
    const label = name || id || 'tool';
    return `${label}: ${success}${content ? ` · ${content}` : ''}`;
  }
  if (event.event_type === 'done') {
    const status = String(payload.status || '').trim();
    const error = String(payload.error || '').trim();
    return error ? `${status || 'done'} · ${error}` : status || 'done';
  }
  if (event.event_type === 'turn') {
    return `${String(payload.current || '')}/${String(payload.max || '')}`;
  }
  return JSON.stringify(payload);
}

function severityClass(severity: 'error' | 'warn' | 'info'): string {
  if (severity === 'error') return 'border-status-error/40 bg-status-error/10';
  if (severity === 'warn') return 'border-status-warning/40 bg-status-warning/10';
  return 'border-border/70 bg-muted/20';
}

export function DigitalAvatarSection({ teamId, canManage }: DigitalAvatarSectionProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const navigate = useNavigate();
  const isCompactInspectorLayout = useMediaQuery('(max-width: 1023px)');
  const { isConversationMode } = useMobileInteractionMode();
  const isConversationCompactLayout = isCompactInspectorLayout && isConversationMode;

  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [publishingAvatar, setPublishingAvatar] = useState(false);
  const [updatingPublicConfig, setUpdatingPublicConfig] = useState(false);
  const [savingNarrativeConfig, setSavingNarrativeConfig] = useState(false);
  const [publicNarrativeDialogOpen, setPublicNarrativeDialogOpen] = useState(false);
  const [publishModeGuideOpen, setPublishModeGuideOpen] = useState(false);
  const [tab, setTab] = useState<WorkspaceTab>('workspace');
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>('overview');
  const [inspectorOpen, setInspectorOpen] = useState<boolean>(() => {
    if (typeof window === 'undefined') return true;
    return !window.matchMedia('(max-width: 1023px)').matches;
  });
  const [mobileAvatarPickerOpen, setMobileAvatarPickerOpen] = useState(false);
  const [activeMobilePanel, setActiveMobilePanel] = useState<MobileWorkspacePanel>(null);
  const [focusMode, setFocusMode] = useState(false);
  const [workspaceChromeCollapsed, setWorkspaceChromeCollapsed] = useState<boolean>(() => {
    if (typeof window === 'undefined') return true;
    const saved = window.localStorage.getItem(`${WORKSPACE_CHROME_STORAGE_PREFIX}default`);
    return saved ? saved === '1' : true;
  });
  const [filter, setFilter] = useState<AvatarFilter>('all');
  const [avatars, setAvatars] = useState<PortalSummary[]>([]);
  const [avatarProjectionMap, setAvatarProjectionMap] = useState<Record<string, AvatarInstanceProjection>>({});
  const [selectedAvatarId, setSelectedAvatarId] = useState<string | null>(null);
  const [selectedAvatar, setSelectedAvatar] = useState<PortalDetail | null>(null);
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [governance, setGovernance] = useState<AvatarGovernanceState>(createEmptyGovernanceState());
  const [governanceConfig, setGovernanceConfig] = useState<AvatarGovernanceAutomationConfig>(DEFAULT_AUTOMATION_CONFIG);
  const [governanceEvents, setGovernanceEvents] = useState<AvatarGovernanceEventPayload[]>([]);
  const [governanceQueue, setGovernanceQueue] = useState<AvatarGovernanceQueueItemPayload[]>([]);
  const [governanceQueueLoaded, setGovernanceQueueLoaded] = useState(false);
  const governanceRef = useRef(governance);
  const selectedAvatarRef = useRef<PortalDetail | null>(selectedAvatar);
  const selectedAvatarIdRef = useRef<string | null>(selectedAvatarId);
  const avatarDetailRequestSeqRef = useRef(0);
  const governancePersistQueueRef = useRef<Promise<void>>(Promise.resolve());
  const governancePersistInFlightRef = useRef(0);
  const [, setSavingGovernance] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [createManagerOpen, setCreateManagerOpen] = useState(false);
  const [managerSessionId, setManagerSessionId] = useState<string | null>(null);
  const [managerProcessing, setManagerProcessing] = useState(false);
  const previousManagerProcessingRef = useRef(false);
  const [runtimeLogFilter, setRuntimeLogFilter] = useState<RuntimeLogFilter>('pending');
  const [governanceKindFilter, setGovernanceKindFilter] = useState<GovernanceKindFilter>('all');
  const [governanceRiskFilter, setGovernanceRiskFilter] = useState<GovernanceRiskFilter>('all');
  const [governanceSearch, setGovernanceSearch] = useState('');
  const [governanceManualOnly, setGovernanceManualOnly] = useState(false);
  const [bootstrapManagerAgentId, setBootstrapManagerAgentId] = useState('');
  const [managerComposeRequest, setManagerComposeRequest] = useState<ChatInputComposeRequest | null>(null);
  const [workspaceDocuments, setWorkspaceDocuments] = useState<DocumentSummary[]>([]);
  const [, setWorkspaceDocumentsLoading] = useState(false);
  const [permissionSelectorDialog, setPermissionSelectorDialog] = useState<'extensions' | 'skills' | null>(null);
  const [showPermissionDocPicker, setShowPermissionDocPicker] = useState(false);
  const [permissionSelectedDocIds, setPermissionSelectedDocIds] = useState<string[]>([]);
  const [permissionSelectedDocumentMap, setPermissionSelectedDocumentMap] = useState<Map<string, DocumentSummary>>(
    new Map(),
  );
  const [permissionSelectedExtensions, setPermissionSelectedExtensions] = useState<string[]>([]);
  const [permissionSelectedSkillIds, setPermissionSelectedSkillIds] = useState<string[]>([]);
  const [permissionExtensionsDirty, setPermissionExtensionsDirty] = useState(false);
  const [permissionSkillsDirty, setPermissionSkillsDirty] = useState(false);
  const [permissionDocumentAccessMode, setPermissionDocumentAccessMode] = useState<PortalDocumentAccessMode>('read_only');
  const [savingPermissionConfig, setSavingPermissionConfig] = useState(false);
  const [publishHeroIntro, setPublishHeroIntro] = useState('');
  const [publishHeroUseCasesText, setPublishHeroUseCasesText] = useState('');
  const [publishHeroWorkingStyle, setPublishHeroWorkingStyle] = useState('');
  const [publishHeroCtaHint, setPublishHeroCtaHint] = useState('');
  const [autoProposalTriggerCountDraft, setAutoProposalTriggerCountDraft] = useState(3);
  const [savingAutomationConfig, setSavingAutomationConfig] = useState(false);
  const [publishViewMode, setPublishViewMode] = useState<PublishViewMode>('visitor');
  const [persistedEvents, setPersistedEvents] = useState<ChatSessionEvent[]>([]);
  const persistedEventsRef = useRef<ChatSessionEvent[]>([]);
  const [persistedEventsLoading, setPersistedEventsLoading] = useState(false);
  const [persistedEventsLoadingMore, setPersistedEventsLoadingMore] = useState(false);
  const [persistedEventsError, setPersistedEventsError] = useState<string | null>(null);
  const [persistedEventFilter, setPersistedEventFilter] = useState<PersistedEventFilter>('all');
  const [persistedEventSearch, setPersistedEventSearch] = useState('');
  const [persistedEventsHasMore, setPersistedEventsHasMore] = useState(false);
  const persistedOldestEventIdRef = useRef<number | null>(null);
  const persistedLatestEventIdRef = useRef<number | null>(null);
  const scheduledGovernanceExecutionRef = useRef<GovernanceExecutionBinding[]>([]);
  const inflightGovernanceExecutionRef = useRef<GovernanceExecutionBinding[]>([]);
  const handledGovernanceActionIdsRef = useRef<Set<string>>(new Set());
  const runtimeAssistantBufferRef = useRef('');
  const wasCompactInspectorLayoutRef = useRef(isCompactInspectorLayout);

  const sessionStoragePrefix = `digital_avatar_manager_session:v2:${teamId}:`;
  const pendingManagerComposeStoragePrefix = `${MANAGER_COMPOSE_STORAGE_PREFIX}${teamId}:`;
  const pendingManagerFocusStorageKey = `${MANAGER_FOCUS_STORAGE_PREFIX}${teamId}`;

  useEffect(() => {
    if (isCompactInspectorLayout && !wasCompactInspectorLayoutRef.current) {
      setInspectorOpen(false);
    }
    wasCompactInspectorLayoutRef.current = isCompactInspectorLayout;
  }, [isCompactInspectorLayout]);

  useEffect(() => {
    if (!isCompactInspectorLayout) {
      setMobileAvatarPickerOpen(false);
    }
  }, [isCompactInspectorLayout]);

  useEffect(() => {
    if (!isConversationCompactLayout) {
      setActiveMobilePanel(null);
      return;
    }
    setMobileAvatarPickerOpen(activeMobilePanel === 'avatar-switcher');
    setInspectorOpen(activeMobilePanel === 'console');
    setTab(activeMobilePanel === 'guide' ? 'guide' : 'workspace');
  }, [activeMobilePanel, isConversationCompactLayout]);

  useEffect(() => {
    if (!PRIMARY_INSPECTOR_TABS.includes(inspectorTab)) {
      setInspectorTab('overview');
    }
  }, [inspectorTab]);

  useEffect(() => {
    selectedAvatarIdRef.current = selectedAvatarId;
  }, [selectedAvatarId]);

  const selectedAvatarListItem = useMemo(
    () => (selectedAvatarId ? avatars.find((avatar) => avatar.id === selectedAvatarId) || null : null),
    [avatars, selectedAvatarId],
  );
  const selectedAvatarDisplay = useMemo(
    () => (selectedAvatar?.id === selectedAvatarId ? selectedAvatar : selectedAvatarListItem),
    [selectedAvatar, selectedAvatarId, selectedAvatarListItem],
  );
  const globalModeActive = !selectedAvatarDisplay;

  const managerAgentId = selectedAvatarDisplay?.codingAgentId || selectedAvatarDisplay?.agentId || null;
  const managerGroupOptions = useMemo(
    () => resolveManagerGroupCandidates(agents, avatars),
    [agents, avatars],
  );

  const fallbackManagerAgentId = managerGroupOptions[0]?.id || null;
  const selectedManagerGroupId = bootstrapManagerAgentId || fallbackManagerAgentId;
  const managerScopedAvatars = useMemo(() => {
    const scopeManagerId = selectedAvatarDisplay
      ? (selectedAvatarDisplay.codingAgentId || selectedAvatarDisplay.agentId || null)
      : selectedManagerGroupId;
    if (!scopeManagerId) return avatars;
    return avatars.filter((avatar) => getDigitalAvatarManagerId(avatar) === scopeManagerId);
  }, [avatars, selectedAvatarDisplay, selectedManagerGroupId]);

  const visibleAvatars = useMemo(() => {
    const base = managerScopedAvatars;
    if (filter === 'all') return base;
    return base.filter((avatar) => detectAvatarType(avatar, avatarProjectionMap[avatar.id]) === filter);
  }, [avatarProjectionMap, filter, managerScopedAvatars]);
  const avatarSections = useMemo(() => {
    if (filter !== 'all') {
      return [
        {
          key: filter,
          title:
            filter === 'external'
              ? t('digitalAvatar.filters.external')
              : t('digitalAvatar.filters.internal'),
          items: visibleAvatars,
        },
      ];
    }

    const externalPublished = visibleAvatars.filter((avatar) => {
      const type = detectAvatarType(avatar, avatarProjectionMap[avatar.id]);
      const status = normalizeAvatarStatus(avatar);
      return type === 'external' && !['draft', 'disabled', 'archived'].includes(status);
    });
    const internalPublished = visibleAvatars.filter((avatar) => {
      const type = detectAvatarType(avatar, avatarProjectionMap[avatar.id]);
      const status = normalizeAvatarStatus(avatar);
      return type === 'internal' && !['draft', 'disabled', 'archived'].includes(status);
    });
    const draftPaused = visibleAvatars.filter((avatar) => {
      const type = detectAvatarType(avatar, avatarProjectionMap[avatar.id]);
      const status = normalizeAvatarStatus(avatar);
      return ['draft', 'disabled', 'archived'].includes(status) || type === 'unknown';
    });

    return [
      {
        key: 'external',
        title: t('digitalAvatar.list.externalSection', '对外服务'),
        items: externalPublished,
      },
      {
        key: 'internal',
        title: t('digitalAvatar.list.internalSection', '对内执行'),
        items: internalPublished,
      },
      {
        key: 'draft',
        title: t('digitalAvatar.list.draftSection', '草稿 / 已归档'),
        items: draftPaused,
      },
    ].filter((section) => section.items.length > 0);
  }, [avatarProjectionMap, filter, t, visibleAvatars]);
  const effectiveManagerAgentId = selectedAvatarDisplay
    ? managerAgentId
    : selectedManagerGroupId;
  const managerAgent = useMemo(
    () => agents.find(agent => agent.id === effectiveManagerAgentId) || null,
    [agents, effectiveManagerAgentId]
  );
  const managerConversationAgentName = useMemo(() => {
    return getAgentDisplayName(managerAgent, t('digitalAvatar.labels.managerAgent'));
  }, [managerAgent, selectedAvatarDisplay, t]);
  const selectedAvatarServiceAgentId = selectedAvatarDisplay?.serviceAgentId || selectedAvatarDisplay?.agentId || null;
  const selectedAvatarServiceAgent = useMemo(
    () => agents.find((agent) => agent.id === selectedAvatarServiceAgentId) || null,
    [agents, selectedAvatarServiceAgentId],
  );

  const selectedAvatarType = selectedAvatarDisplay
    ? detectAvatarType(selectedAvatarDisplay, avatarProjectionMap[selectedAvatarDisplay.id])
    : 'unknown';
  const selectedAvatarStatus = useMemo(
    () => normalizeAvatarStatus(selectedAvatar || selectedAvatarDisplay),
    [selectedAvatar, selectedAvatarDisplay],
  );
  const selectedAvatarEffectivePublicConfig =
    selectedAvatar?.effectivePublicConfig
    || selectedAvatarDisplay?.effectivePublicConfig
    || null;
  const selectedAvatarDocumentAccessMode =
    selectedAvatarEffectivePublicConfig?.effectiveDocumentAccessMode
    || selectedAvatar?.documentAccessMode
    || selectedAvatarDisplay?.documentAccessMode;
  const selectedAvatarShowChatWidget = useMemo(() => {
    const raw = (selectedAvatar?.settings as Record<string, unknown> | undefined)?.showChatWidget;
    return typeof raw === 'boolean' ? raw : true;
  }, [selectedAvatar?.settings]);
  const selectedAvatarShowBoundDocuments = useMemo(() => {
    const raw = (selectedAvatar?.settings as Record<string, unknown> | undefined)?.showBoundDocuments;
    if (typeof raw === 'boolean') return raw;
    return selectedAvatarEffectivePublicConfig?.showBoundDocuments ?? true;
  }, [selectedAvatar?.settings, selectedAvatarEffectivePublicConfig?.showBoundDocuments]);
  const selectedAvatarEnabledExtensionEntries = useMemo(
    () => getEnabledExtensionEntries(selectedAvatarServiceAgent),
    [selectedAvatarServiceAgent],
  );
  const selectedAvatarAssignedSkillEntries = useMemo(
    () => getAssignedSkillEntries(selectedAvatarServiceAgent),
    [selectedAvatarServiceAgent],
  );
  const selectedAvatarEffectiveExtensionEntries = useMemo(() => {
    const effectiveIds = selectedAvatarEffectivePublicConfig?.effectiveAllowedExtensions;
    if (
      effectiveIds
      && (effectiveIds.length > 0 || selectedAvatarEffectivePublicConfig?.extensionsInherited === false)
    ) {
      return effectiveIds.map((id) => ({
        id,
        name: resolveExtensionLabel(id),
      }));
    }
    return selectedAvatarEnabledExtensionEntries;
  }, [
    selectedAvatarEffectivePublicConfig?.effectiveAllowedExtensions,
    selectedAvatarEffectivePublicConfig?.extensionsInherited,
    selectedAvatarEnabledExtensionEntries,
  ]);
  const selectedAvatarEffectiveSkillEntries = useMemo(() => {
    const effectiveIds = selectedAvatarEffectivePublicConfig?.effectiveAllowedSkillIds;
    if (
      effectiveIds
      && (effectiveIds.length > 0 || selectedAvatarEffectivePublicConfig?.skillsInherited === false)
    ) {
      return effectiveIds.map((id) => {
        const match = selectedAvatarAssignedSkillEntries.find((item) => item.id === id);
        return {
          id,
          name: match?.name || id,
        };
      });
    }
    return selectedAvatarAssignedSkillEntries;
  }, [
    selectedAvatarAssignedSkillEntries,
    selectedAvatarEffectivePublicConfig?.effectiveAllowedSkillIds,
    selectedAvatarEffectivePublicConfig?.skillsInherited,
  ]);
  const selectedAvatarRuntimeExtensionOptions = useMemo(
    () => getRuntimeExtensionOptions(selectedAvatarServiceAgent),
    [selectedAvatarServiceAgent],
  );
  const workspaceDocumentsById = useMemo(
    () => new Map(workspaceDocuments.map((doc) => [doc.id, doc])),
    [workspaceDocuments],
  );
  const permissionSelectedDocuments = useMemo(
    () =>
      permissionSelectedDocIds
        .map((id) => permissionSelectedDocumentMap.get(id) || workspaceDocumentsById.get(id))
        .filter((doc): doc is DocumentSummary => Boolean(doc)),
    [permissionSelectedDocIds, permissionSelectedDocumentMap, workspaceDocumentsById],
  );
  const permissionSelectedExtensionEntries = useMemo(
    () =>
      selectedAvatarRuntimeExtensionOptions
        .filter((option) => permissionSelectedExtensions.includes(option.id))
        .map((option) => ({
          id: option.id,
          name: option.label,
        })),
    [permissionSelectedExtensions, selectedAvatarRuntimeExtensionOptions],
  );
  const permissionSelectedSkillEntries = useMemo(
    () => selectedAvatarAssignedSkillEntries.filter((entry) => permissionSelectedSkillIds.includes(entry.id)),
    [permissionSelectedSkillIds, selectedAvatarAssignedSkillEntries],
  );
  const selectedAvatarHasPortalRestriction = selectedAvatarEffectivePublicConfig
    ? !(selectedAvatarEffectivePublicConfig.extensionsInherited && selectedAvatarEffectivePublicConfig.skillsInherited)
    : Boolean(
        (selectedAvatarDisplay?.allowedExtensions && selectedAvatarDisplay.allowedExtensions.length > 0)
          || (selectedAvatarDisplay?.allowedSkillIds && selectedAvatarDisplay.allowedSkillIds.length > 0),
      );
  const selectedAvatarCapabilityScopeHint = useMemo(
    () => selectedAvatarHasPortalRestriction
      ? t(
          'digitalAvatar.workspace.capabilityScopeRestricted',
          '当前已按门户白名单收敛，只开放这里列出的扩展与技能。',
        )
      : t(
          'digitalAvatar.workspace.capabilityScopeInherited',
          '当前未额外收敛，按服务分身已启用的扩展与技能对外开放。',
        ),
    [selectedAvatarHasPortalRestriction, t],
  );
  const selectedAvatarStatusLabel = useMemo(() => {
    if (!selectedAvatarStatus) return t('digitalAvatar.labels.unset');
    return avatarStatusLabel(selectedAvatarStatus, t);
  }, [selectedAvatarStatus, t]);
  const selectedAvatarOutputFormLabel = useMemo(
    () => formatPortalOutputForm(selectedAvatarDisplay?.outputForm, t),
    [selectedAvatarDisplay?.outputForm, t],
  );
  const selectedAvatarExposureLabel = useMemo(
    () => formatPublicExposure(selectedAvatarEffectivePublicConfig?.exposure, t),
    [selectedAvatarEffectivePublicConfig?.exposure, t],
  );
  const selectedAvatarChatWidgetEffectLabel = useMemo(() => {
    if (!selectedAvatarEffectivePublicConfig?.chatEnabled) {
      return t('digitalAvatar.workspace.chatUnavailable', '聊天未启用');
    }
    return selectedAvatarEffectivePublicConfig.showChatWidget
      ? t('digitalAvatar.workspace.chatWidgetVisible', '显示默认聊天挂件')
      : t('digitalAvatar.workspace.chatWidgetHidden', '聊天可用，但默认挂件已关闭');
  }, [selectedAvatarEffectivePublicConfig, t]);
  const permissionPreviewDraft = useMemo(
    () => buildPermissionPreview(permissionDocumentAccessMode, t),
    [permissionDocumentAccessMode, t],
  );
  const permissionScopeHint = useMemo(() => {
    const extensionRestricted = permissionSelectedExtensions.length !== selectedAvatarRuntimeExtensionOptions.length;
    const skillRestricted = permissionSelectedSkillIds.length !== selectedAvatarAssignedSkillEntries.length;
    return extensionRestricted || skillRestricted
      ? t(
          'digitalAvatar.workspace.capabilityScopeRestricted',
          '当前已按门户白名单收敛，只开放这里列出的扩展与技能。',
        )
      : t(
          'digitalAvatar.workspace.capabilityScopeInherited',
          '当前未额外收敛，按服务分身已启用的扩展与技能对外开放。',
        );
  }, [
    permissionSelectedExtensions.length,
    permissionSelectedSkillIds.length,
    selectedAvatarAssignedSkillEntries.length,
    selectedAvatarRuntimeExtensionOptions.length,
    t,
  ]);
  const publishPath = selectedAvatarDisplay?.slug ? `/p/${selectedAvatarDisplay.slug}` : '';
  const selectedAvatarPublicUrl = selectedAvatarEffectivePublicConfig?.publicAccessEnabled
    ? (selectedAvatarDisplay?.publicUrl || publishPath || '')
    : '';
  const selectedAvatarPreviewUrl = selectedAvatarDisplay?.previewUrl || '';
  const selectedAvatarTestUrl = selectedAvatarEffectivePublicConfig?.publicAccessEnabled
    ? (selectedAvatarDisplay?.testPublicUrl || '')
    : '';
  const selectedAvatarNarrativeUseCases = useMemo(
    () => splitNarrativeUseCases(publishHeroUseCasesText),
    [publishHeroUseCasesText],
  );
  const selectedAvatarNarrativeConfigured = Boolean(
    publishHeroIntro.trim()
      || selectedAvatarNarrativeUseCases.length > 0
      || publishHeroWorkingStyle.trim()
      || publishHeroCtaHint.trim(),
  );
  const selectedAvatarLastActivityLabel = useMemo(() => {
    if (!selectedAvatarDisplay?.updatedAt) return t('digitalAvatar.labels.unset');
    return formatRelativeTime(selectedAvatarDisplay.updatedAt);
  }, [selectedAvatarDisplay?.updatedAt, t]);
  const availablePublishModes = useMemo(() => {
    const modes: PublishViewMode[] = [];
    if (selectedAvatarPublicUrl) modes.push('visitor');
    if (selectedAvatarPreviewUrl) modes.push('preview');
    if (selectedAvatarTestUrl) modes.push('test');
    return modes;
  }, [selectedAvatarPreviewUrl, selectedAvatarPublicUrl, selectedAvatarTestUrl]);
  const activePublishUrl = publishViewMode === 'preview'
    ? selectedAvatarPreviewUrl
    : publishViewMode === 'test'
    ? selectedAvatarTestUrl
    : selectedAvatarPublicUrl;
  const publishModeDescription = useMemo(() => {
    if (publishViewMode === 'preview') {
      return {
        title: t('digitalAvatar.workspace.publishMode.previewTitle', '管理预览视角'),
        description: t('digitalAvatar.workspace.publishMode.previewDesc', '用于内部检查页面内容、权限边界和对话入口是否按预期展示。'),
        bullets: [
          t('digitalAvatar.workspace.publishMode.previewBullet1', '可用于管理员验收当前分身配置与访客说明。'),
          t('digitalAvatar.workspace.publishMode.previewBullet2', '适合在正式发布前检查权限、文档与提示文案。'),
          t('digitalAvatar.workspace.publishMode.previewBullet3', '如果访客体验异常，优先从这里定位。'),
        ],
      };
    }
    if (publishViewMode === 'test') {
      return {
        title: t('digitalAvatar.workspace.publishMode.testTitle', '测试入口视角'),
        description: t('digitalAvatar.workspace.publishMode.testDesc', '用于联调、网络验证或内网快速访问，不代表正式访客入口。'),
        bullets: [
          t('digitalAvatar.workspace.publishMode.testBullet1', '适合技术测试、端口联调和问题排查。'),
          t('digitalAvatar.workspace.publishMode.testBullet2', '地址可能变化，不建议直接对外分发。'),
          t('digitalAvatar.workspace.publishMode.testBullet3', '正式交付仍以访客入口或管理预览为准。'),
        ],
      };
    }
    return {
      title: t('digitalAvatar.workspace.publishMode.visitorTitle', '访客视角'),
      description: t('digitalAvatar.workspace.publishMode.visitorDesc', '这是外部用户最终看到的数字分身页面，重点是能力说明、边界提示和对话入口。'),
      bullets: [
        t('digitalAvatar.workspace.publishMode.visitorBullet1', '展示分身能做什么、不能做什么，以及当前文档边界。'),
        t('digitalAvatar.workspace.publishMode.visitorBullet2', '展示示例问题、开放能力摘要和右下角对话入口。'),
        t('digitalAvatar.workspace.publishMode.visitorBullet3', '适合交付给客户、合作伙伴或外部访客直接使用。'),
      ],
    };
  }, [publishViewMode, t]);
  const runtimePendingCount = useMemo(
    () => governance.runtimeLogs.filter((item) => item.status === 'pending').length,
    [governance.runtimeLogs]
  );
  const recommendedQuickActions = useMemo(() => {
    if (!selectedAvatar) return [] as Array<{ key: string; kind: 'optimize' | 'setAggressive' | 'setBalanced' | 'setConservative'; label: string }>;
    const next: Array<{ key: string; kind: 'optimize' | 'setAggressive' | 'setBalanced' | 'setConservative'; label: string }> = [];
    if (runtimePendingCount > 0) {
      next.push({
        key: 'optimize',
        kind: 'optimize',
        label: t('digitalAvatar.workspace.quickOptimizeCurrent', '生成优化工单'),
      });
    }
    if (selectedAvatarType === 'internal') {
      next.push({
        key: 'setAggressive',
        kind: 'setAggressive',
        label: t('digitalAvatar.workspace.quickSetAggressive', '阈值激进(3)'),
      });
    } else {
      next.push({
        key: 'setConservative',
        kind: 'setConservative',
        label: t('digitalAvatar.workspace.quickSetConservative', '阈值保守(7)'),
      });
    }
    if (next.length < 2) {
      next.push({
        key: 'setBalanced',
        kind: 'setBalanced',
        label: t('digitalAvatar.workspace.quickSetBalanced', '阈值平衡(5)'),
      });
    }
    return next.slice(0, 2);
  }, [runtimePendingCount, selectedAvatar, selectedAvatarType, t]);

  const loadWorkspaceDocuments = useCallback(async () => {
    const boundIds = selectedAvatar?.boundDocumentIds || [];
    if (boundIds.length === 0) {
      setWorkspaceDocuments([]);
      return;
    }

    setWorkspaceDocumentsLoading(true);
    try {
      const res = await documentApi.listDocuments(teamId, 1, 500);
      const byId = new Map(res.items.map((item) => [item.id, item]));
      const docs = boundIds
        .map((id) => byId.get(id))
        .filter((item): item is DocumentSummary => Boolean(item));
      setWorkspaceDocuments(docs);
    } catch (error) {
      console.error('Failed to load workspace documents:', error);
      setWorkspaceDocuments([]);
    } finally {
      setWorkspaceDocumentsLoading(false);
    }
  }, [selectedAvatar?.boundDocumentIds, teamId]);

  useEffect(() => {
    let cancelled = false;
    loadWorkspaceDocuments().catch((error) => {
      if (!cancelled) {
        console.error('Failed to refresh workspace documents:', error);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [loadWorkspaceDocuments]);

  useEffect(() => {
    if (!selectedAvatar) {
      setPermissionSelectedDocIds([]);
      setPermissionSelectedDocumentMap(new Map());
      setPermissionSelectedExtensions([]);
      setPermissionSelectedSkillIds([]);
      setPermissionExtensionsDirty(false);
      setPermissionSkillsDirty(false);
      setPermissionDocumentAccessMode('read_only');
      return;
    }

    setPermissionSelectedDocIds(selectedAvatar.boundDocumentIds || []);
    setPermissionSelectedDocumentMap(new Map());
    setPermissionDocumentAccessMode(selectedAvatar.documentAccessMode || 'read_only');
    setPermissionSelectedExtensions(
      selectedAvatar.allowedExtensions !== undefined && selectedAvatar.allowedExtensions !== null
        ? selectedAvatar.allowedExtensions
        : selectedAvatarRuntimeExtensionOptions.map((item) => item.id),
    );
    setPermissionSelectedSkillIds(
      selectedAvatar.allowedSkillIds !== undefined && selectedAvatar.allowedSkillIds !== null
        ? selectedAvatar.allowedSkillIds
        : selectedAvatarAssignedSkillEntries.map((item) => item.id),
    );
    setPermissionExtensionsDirty(false);
    setPermissionSkillsDirty(false);
  }, [selectedAvatar, selectedAvatarAssignedSkillEntries, selectedAvatarRuntimeExtensionOptions]);

  useEffect(() => {
    if (!selectedAvatar) {
      setPublishHeroIntro('');
      setPublishHeroUseCasesText('');
      setPublishHeroWorkingStyle('');
      setPublishHeroCtaHint('');
      return;
    }
    const narrative = readAvatarPublicNarrative(
      (selectedAvatar.settings as Record<string, unknown> | undefined) || undefined,
    );
    setPublishHeroIntro(narrative.heroIntro || '');
    setPublishHeroUseCasesText(joinNarrativeUseCases(narrative.heroUseCases));
    setPublishHeroWorkingStyle(narrative.heroWorkingStyle || '');
    setPublishHeroCtaHint(narrative.heroCtaHint || '');
  }, [selectedAvatar]);

  useEffect(() => {
    setAutoProposalTriggerCountDraft(governanceConfig.autoProposalTriggerCount);
  }, [governanceConfig.autoProposalTriggerCount]);

  useEffect(() => {
    setInspectorTab('overview');
  }, [selectedAvatarId]);

  useEffect(() => {
    if (!availablePublishModes.includes(publishViewMode)) {
      setPublishViewMode(availablePublishModes[0] || 'visitor');
    }
  }, [availablePublishModes, publishViewMode]);

  const displayPersistedEvents = useMemo(
    () => mergePersistedEventsForDisplay(persistedEvents),
    [persistedEvents],
  );

  const visiblePersistedEvents = useMemo(() => {
    const keyword = persistedEventSearch.trim().toLowerCase();
    return displayPersistedEvents.filter((event) => {
      if (persistedEventFilter === 'error' && eventSeverity(event) !== 'error') return false;
      if (persistedEventFilter === 'tool' && !['toolcall', 'toolresult'].includes(event.event_type)) return false;
      if (persistedEventFilter === 'thinking' && !['thinking', 'turn', 'compaction'].includes(event.event_type)) return false;
      if (persistedEventFilter === 'status' && !['status', 'done', 'workspace_changed'].includes(event.event_type)) return false;
      if (!keyword) return true;
      const text = `${event.event_type} ${eventSummary(event)}`.toLowerCase();
      return text.includes(keyword);
    });
  }, [displayPersistedEvents, persistedEventFilter, persistedEventSearch]);

  const derivedGovernanceAuditRows = useMemo(() => {
    const rows: Array<{
      id: string;
      ts: string;
      type: 'capability' | 'proposal' | 'ticket';
      title: string;
      status: string;
      detail: string;
    }> = [];

    governance.capabilityRequests.forEach((item) => {
      if (item.status === 'pending' && !item.decision) return;
      rows.push({
        id: `capability:${item.id}`,
        ts: item.updatedAt || item.createdAt,
        type: 'capability',
        title: item.title,
        status: item.status,
        detail: item.decisionReason || item.detail || '',
      });
    });
    governance.gapProposals.forEach((item) => {
      if (item.status === 'draft') return;
      rows.push({
        id: `proposal:${item.id}`,
        ts: item.updatedAt || item.createdAt,
        type: 'proposal',
        title: item.title,
        status: item.status,
        detail: item.description || '',
      });
    });
    governance.optimizationTickets.forEach((item) => {
      if (item.status === 'pending') return;
      rows.push({
        id: `ticket:${item.id}`,
        ts: item.updatedAt || item.createdAt,
        type: 'ticket',
        title: item.title,
        status: item.status,
        detail: item.proposal || item.evidence || '',
      });
    });

    rows.sort((a, b) => {
      const ta = Date.parse(a.ts);
      const tb = Date.parse(b.ts);
      return (Number.isFinite(tb) ? tb : 0) - (Number.isFinite(ta) ? ta : 0);
    });
    return rows;
  }, [governance.capabilityRequests, governance.gapProposals, governance.optimizationTickets]);
  const governanceAuditRows = useMemo(() => {
    if (governanceEvents.length === 0) {
      return derivedGovernanceAuditRows;
    }

    return governanceEvents
      .filter((event) => event.entity_type !== 'runtime')
      .map((event) => ({
        id: event.event_id || `${event.entity_type}:${event.entity_id || event.created_at}`,
        ts: event.created_at,
        type: (event.entity_type === 'capability' || event.entity_type === 'proposal' || event.entity_type === 'ticket'
          ? event.entity_type
          : 'ticket') as 'capability' | 'proposal' | 'ticket',
        title: event.title,
        status: event.status || event.event_type,
        detail: [
          event.detail,
          event.actor_name ? t('digitalAvatar.governance.actorMeta', '执行人：{{name}}', { name: event.actor_name }) : '',
        ].filter(Boolean).join(' · '),
      }))
      .sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts));
  }, [derivedGovernanceAuditRows, governanceEvents]);

  const derivedGovernanceTimelineRows = useMemo(() => {
    const rows: Array<{
      id: string;
      ts: string;
      rowType: 'runtime' | 'capability' | 'proposal' | 'ticket';
      title: string;
      detail: string;
      status: string;
      meta: string[];
      runtimeId?: string;
    }> = [];

    governance.runtimeLogs.forEach((item) => {
      rows.push({
        id: `runtime:${item.id}`,
        ts: item.createdAt,
        rowType: 'runtime',
        title: item.title,
        detail: item.proposal || item.evidence || '',
        status: item.status,
        meta: [item.problemType, item.risk].filter(Boolean),
        runtimeId: item.id,
      });
    });

    governanceAuditRows.forEach((item) => {
      rows.push({
        id: item.id,
        ts: item.ts,
        rowType: item.type,
        title: item.title,
        detail: item.detail,
        status: item.status,
        meta: [],
      });
    });

    rows.sort((a, b) => {
      const ta = Date.parse(a.ts);
      const tb = Date.parse(b.ts);
      return (Number.isFinite(tb) ? tb : 0) - (Number.isFinite(ta) ? ta : 0);
    });

    return rows;
  }, [governance.runtimeLogs, derivedGovernanceAuditRows]);

  const governanceTimelineRows = useMemo(() => {
    if (governanceEvents.length === 0) {
      return derivedGovernanceTimelineRows;
    }

    return governanceEvents
      .map((event) => {
        const actorMeta = event.actor_name ? [t('digitalAvatar.governance.actorMeta', '执行人：{{name}}', { name: event.actor_name })] : [];
        const eventMeta = event.event_type ? [t('digitalAvatar.governance.eventMeta', '事件：{{type}}', { type: event.event_type })] : [];
        const rowType = event.entity_type === 'runtime'
          ? 'runtime'
          : event.entity_type === 'capability'
          ? 'capability'
          : event.entity_type === 'proposal'
          ? 'proposal'
          : 'ticket';
        return {
          id: event.event_id || `${event.entity_type}:${event.entity_id || event.created_at}`,
          ts: event.created_at,
          rowType: rowType as 'runtime' | 'capability' | 'proposal' | 'ticket',
          title: event.title,
          detail: event.detail || '',
          status: event.status || event.event_type,
          meta: [...eventMeta, ...actorMeta],
          runtimeId: rowType === 'runtime' ? event.entity_id || undefined : undefined,
        };
      })
      .sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts));
  }, [derivedGovernanceTimelineRows, governanceEvents]);

  const visibleGovernanceTimelineRows = useMemo(() => {
    if (runtimeLogFilter === 'all') return governanceTimelineRows;
    return governanceTimelineRows.filter((item) =>
      ['pending', 'needs_human', 'pending_approval', 'approved', 'pilot', 'experimenting'].includes(item.status),
    );
  }, [governanceTimelineRows, runtimeLogFilter]);
  const derivedGovernanceQueueItems = useMemo(() => {
    const rows: Array<{
      id: string;
      kind: 'capability' | 'proposal' | 'ticket';
      title: string;
      detail: string;
      status: string;
      ts: string;
      meta: string[];
      sourceId: string;
    }> = [];

    governance.capabilityRequests.forEach((item) => {
      if (!['pending', 'needs_human'].includes(item.status)) return;
      rows.push({
        id: `queue:capability:${item.id}`,
        kind: 'capability',
        title: item.title,
        detail: item.detail || item.decisionReason || '',
        status: item.status,
        ts: item.updatedAt || item.createdAt,
        meta: [item.risk, item.source].filter(Boolean),
        sourceId: item.id,
      });
    });

    governance.gapProposals.forEach((item) => {
      if (!['pending_approval', 'approved', 'pilot'].includes(item.status)) return;
      rows.push({
        id: `queue:proposal:${item.id}`,
        kind: 'proposal',
        title: item.title,
        detail: item.description || '',
        status: item.status,
        ts: item.updatedAt || item.createdAt,
        meta: [
          item.expectedGain,
          item.decisionReason
            ? t('digitalAvatar.governance.decisionReasonMeta', '决策说明：{{reason}}', {
                reason: item.decisionReason,
              })
            : '',
        ].filter(Boolean),
        sourceId: item.id,
      });
    });

    governance.optimizationTickets.forEach((item) => {
      if (!['pending', 'approved', 'experimenting'].includes(item.status)) return;
      rows.push({
        id: `queue:ticket:${item.id}`,
        kind: 'ticket',
        title: item.title,
        detail: item.proposal || item.evidence || '',
        status: item.status,
        ts: item.updatedAt || item.createdAt,
        meta: [
          item.problemType,
          item.risk,
          item.decisionReason
            ? t('digitalAvatar.governance.decisionReasonMeta', '决策说明：{{reason}}', {
                reason: item.decisionReason,
              })
            : '',
        ].filter(Boolean),
        sourceId: item.id,
      });
    });

    rows.sort((a, b) => {
      const ta = Date.parse(a.ts);
      const tb = Date.parse(b.ts);
      return (Number.isFinite(tb) ? tb : 0) - (Number.isFinite(ta) ? ta : 0);
    });

    return rows;
  }, [governance.capabilityRequests, governance.gapProposals, governance.optimizationTickets]);

  const governanceQueueItems = useMemo(() => {
    if (governanceQueueLoaded) {
      return governanceQueue.map((item) => ({
        id: item.id,
        kind: (item.kind === 'capability' || item.kind === 'proposal' || item.kind === 'ticket'
          ? item.kind
          : 'ticket') as 'capability' | 'proposal' | 'ticket',
        title: item.title,
        detail: item.detail,
        status: item.status,
        ts: item.ts,
        meta: item.meta || [],
        sourceId: item.source_id,
      }));
    }
    return derivedGovernanceQueueItems;
  }, [derivedGovernanceQueueItems, governanceQueue, governanceQueueLoaded]);
  const governanceFilterActive = governanceKindFilter !== 'all'
    || governanceRiskFilter !== 'all'
    || governanceSearch.trim().length > 0
    || governanceManualOnly;
  const governanceSearchKeyword = governanceSearch.trim().toLowerCase();
  const rawPendingGovernanceQueueItems = useMemo(
    () => governanceQueueItems.filter((item) => isOpenGovernanceStatus(item.status)),
    [governanceQueueItems]
  );
  const rawResolvedGovernanceQueueItems = useMemo(
    () => governanceQueueItems.filter((item) => !isOpenGovernanceStatus(item.status)),
    [governanceQueueItems]
  );
  const pendingGovernanceQueueItems = useMemo(
    () => rawPendingGovernanceQueueItems.filter((item) => {
      if (governanceKindFilter !== 'all' && governanceKindFilter !== item.kind) return false;
      if (governanceManualOnly && !isHumanReviewQueueItem(item.kind, item.status)) return false;
      if (governanceRiskFilter !== 'all' && extractRiskFromTexts(item.meta) !== governanceRiskFilter) return false;
      if (!governanceSearchKeyword) return true;
      const text = `${item.title} ${item.detail} ${item.meta.join(' ')}`.toLowerCase();
      return text.includes(governanceSearchKeyword);
    }),
    [governanceKindFilter, governanceManualOnly, governanceRiskFilter, governanceSearchKeyword, rawPendingGovernanceQueueItems]
  );
  const resolvedGovernanceQueueItems = useMemo(
    () => rawResolvedGovernanceQueueItems.filter((item) => {
      if (governanceKindFilter !== 'all' && governanceKindFilter !== item.kind) return false;
      if (governanceManualOnly && !isHumanReviewQueueItem(item.kind, item.status)) return false;
      if (governanceRiskFilter !== 'all' && extractRiskFromTexts(item.meta) !== governanceRiskFilter) return false;
      if (!governanceSearchKeyword) return true;
      const text = `${item.title} ${item.detail} ${item.meta.join(' ')}`.toLowerCase();
      return text.includes(governanceSearchKeyword);
    }),
    [governanceKindFilter, governanceManualOnly, governanceRiskFilter, governanceSearchKeyword, rawResolvedGovernanceQueueItems]
  );
  const automatedGovernanceRows = useMemo(
    () => governanceTimelineRows
      .filter((item) => {
        if (item.rowType !== 'runtime') return false;
        if (governanceManualOnly) return false;
        if (governanceKindFilter !== 'all' && governanceKindFilter !== 'runtime') return false;
        if (governanceRiskFilter !== 'all' && extractRiskFromTexts(item.meta) !== governanceRiskFilter) return false;
        if (!governanceSearchKeyword) return true;
        const text = `${item.title} ${item.detail} ${item.meta.join(' ')}`.toLowerCase();
        return text.includes(governanceSearchKeyword);
      })
      .slice(0, 8),
    [governanceKindFilter, governanceManualOnly, governanceRiskFilter, governanceSearchKeyword, governanceTimelineRows]
  );

  const runtimeSuggestionText = useMemo<RuntimeSuggestionText>(() => ({
    unknownTool: t('digitalAvatar.governance.runtimeUnknownTool', '未知工具'),
    toolFailureTitle: (tool: string) =>
      t('digitalAvatar.governance.runtimeToolFailureTitle', '工具执行失败：{{tool}}', { tool }),
    toolFailureEvidenceFallback: t(
      'digitalAvatar.governance.runtimeToolFailureEvidenceFallback',
      '工具执行失败，未返回详细预览。'
    ),
    toolFailureProposal: (tool: string) =>
      t(
        'digitalAvatar.governance.runtimeToolFailureProposal',
        '检查 {{tool}} 的权限边界、输入契约与回退路径，并补充有停止条件的受控重试策略。',
        { tool }
      ),
    toolFailureGain: t(
      'digitalAvatar.governance.runtimeToolFailureGain',
      '降低重复工具失败率，提升任务完成稳定性。'
    ),
    sessionFailedTitle: t('digitalAvatar.governance.runtimeSessionFailedTitle', '会话终止失败'),
    sessionFailedProposal: t(
      'digitalAvatar.governance.runtimeSessionFailedProposal',
      '复核任务提示词与策略约束，补充最小失败恢复流程。'
    ),
    sessionFailedGain: t(
      'digitalAvatar.governance.runtimeSessionFailedGain',
      '降低硬失败中断，提升成功交付率。'
    ),
  }), [t]);

  useEffect(() => {
    governanceRef.current = governance;
  }, [governance]);

  useEffect(() => {
    selectedAvatarRef.current = selectedAvatar;
  }, [selectedAvatar]);

  useEffect(() => {
    persistedEventsRef.current = persistedEvents;
  }, [persistedEvents]);

  const loadAvatars = useCallback(async (withLoading = true) => {
    try {
      if (withLoading) setLoading(true);
      setRefreshing(true);
      const [portalRes, agentRes, avatarProjectionRes] = await Promise.all([
        avatarPortalApi.list(teamId, 1, 200),
        listAllTeamAgents(teamId),
        avatarPortalApi.listInstances(teamId).catch(() => []),
      ]);
      const avatarItems = (portalRes.items || []).filter(isAvatar);
      const nextAgents = agentRes || [];
      const nextProjectionMap = Object.fromEntries(
        (avatarProjectionRes || []).map((item) => [item.portalId, item]),
      );
      const managerGroups = resolveManagerGroupCandidates(nextAgents, avatarItems);
      setAvatars(avatarItems);
      setAvatarProjectionMap(nextProjectionMap);
      setAgents(nextAgents);
      setBootstrapManagerAgentId((prev) => {
        if (prev && managerGroups.some(agent => agent.id === prev)) return prev;
        return managerGroups[0]?.id || '';
      });
      setSelectedAvatarId((prev) => {
        if (prev && avatarItems.some(item => item.id === prev)) return prev;
        return null;
      });
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('digitalAvatar.states.loading'));
    } finally {
      if (withLoading) setLoading(false);
      setRefreshing(false);
    }
  }, [addToast, t, teamId]);

  const loadGovernanceEvents = useCallback(async (avatarId: string) => {
    try {
      const events = await avatarPortalApi.listGovernanceEvents(teamId, avatarId, 120);
      setGovernanceEvents(events);
    } catch {
      setGovernanceEvents([]);
    }
  }, [teamId]);

  const loadGovernanceQueue = useCallback(async (avatarId: string) => {
    try {
      const queue = await avatarPortalApi.listGovernanceQueue(teamId, avatarId);
      setGovernanceQueue(queue);
      setGovernanceQueueLoaded(true);
    } catch {
      setGovernanceQueue([]);
      setGovernanceQueueLoaded(false);
    }
  }, [teamId]);

  const loadAvatarDetail = useCallback(async (avatarId: string) => {
    const requestSeq = avatarDetailRequestSeqRef.current + 1;
    avatarDetailRequestSeqRef.current = requestSeq;
    try {
      const [detail, governancePayload, governanceEventPayload, governanceQueuePayload] = await Promise.all([
        avatarPortalApi.get(teamId, avatarId),
        avatarPortalApi.getGovernance(teamId, avatarId).catch(() => null),
        avatarPortalApi.listGovernanceEvents(teamId, avatarId, 120).catch(() => []),
        avatarPortalApi.listGovernanceQueue(teamId, avatarId).catch(() => null),
      ]);
      if (avatarDetailRequestSeqRef.current !== requestSeq || selectedAvatarIdRef.current !== avatarId) {
        return;
      }
      setSelectedAvatar(detail);
      setGovernanceEvents(governanceEventPayload);
      setGovernanceQueue(governanceQueuePayload || []);
      setGovernanceQueueLoaded(Array.isArray(governanceQueuePayload));
      const nextGovernance = governancePayload
        ? readGovernanceState({ digitalAvatarGovernance: governancePayload.state })
        : readGovernanceState(detail.settings);
      const nextGovernanceConfig = governancePayload
        ? readGovernanceAutomationConfig({ digitalAvatarGovernanceConfig: governancePayload.config })
        : readGovernanceAutomationConfig(detail.settings);
      setGovernance(nextGovernance);
      setGovernanceConfig(nextGovernanceConfig);
      setRuntimeLogFilter('pending');
      try {
        const saved = window.localStorage.getItem(`${sessionStoragePrefix}${avatarId}`);
        if (!saved) {
          setManagerSessionId(null);
        } else {
          try {
            const session = await chatApi.getSession(saved);
            if (avatarDetailRequestSeqRef.current !== requestSeq || selectedAvatarIdRef.current !== avatarId) {
              return;
            }
            if (session.portal_id === avatarId) {
              setManagerSessionId(saved);
            } else {
              window.localStorage.removeItem(`${sessionStoragePrefix}${avatarId}`);
              setManagerSessionId(null);
            }
          } catch {
            try {
              window.localStorage.removeItem(`${sessionStoragePrefix}${avatarId}`);
            } catch {}
            setManagerSessionId(null);
          }
        }
      } catch {
        setManagerSessionId(null);
      }
    } catch (err) {
      if (avatarDetailRequestSeqRef.current !== requestSeq || selectedAvatarIdRef.current !== avatarId) {
        return;
      }
      addToast('error', err instanceof Error ? err.message : t('common.error'));
      setSelectedAvatar(null);
      setGovernanceEvents([]);
      setGovernanceQueue([]);
      setGovernanceQueueLoaded(false);
      setGovernance(createEmptyGovernanceState());
      setGovernanceConfig(DEFAULT_AUTOMATION_CONFIG);
      setManagerSessionId(null);
      setRuntimeLogFilter('pending');
    }
  }, [addToast, teamId, t, sessionStoragePrefix]);

  const handleSaveAvatarPermissions = useCallback(async () => {
    if (!selectedAvatar || !canManage || savingPermissionConfig) {
      return;
    }

    try {
      setSavingPermissionConfig(true);
      const request: Parameters<typeof avatarPortalApi.update>[2] = {
        boundDocumentIds: permissionSelectedDocIds,
        documentAccessMode: permissionDocumentAccessMode,
      };
      if (permissionExtensionsDirty) {
        const inheritedExtensions = selectedAvatarRuntimeExtensionOptions.map((item) => item.id);
        const currentExtensions = selectedAvatar.allowedExtensions ?? inheritedExtensions;
        if (!sameStringSelection(permissionSelectedExtensions, currentExtensions)) {
          request.allowedExtensions = permissionSelectedExtensions;
        }
      }
      if (permissionSkillsDirty) {
        const inheritedSkills = selectedAvatarAssignedSkillEntries.map((item) => item.id);
        const currentSkills = selectedAvatar.allowedSkillIds ?? inheritedSkills;
        if (!sameStringSelection(permissionSelectedSkillIds, currentSkills)) {
          request.allowedSkillIds = permissionSelectedSkillIds;
        }
      }
      const updated = await avatarPortalApi.update(teamId, selectedAvatar.id, request);
      setSelectedAvatar(updated);
      await loadAvatars(false);
      await loadAvatarDetail(updated.id);
      addToast('success', t('common.saved', '已保存'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSavingPermissionConfig(false);
    }
  }, [
    addToast,
    canManage,
    loadAvatarDetail,
    loadAvatars,
    permissionDocumentAccessMode,
    permissionSelectedDocIds,
    permissionSelectedExtensions,
    permissionSelectedSkillIds,
    permissionExtensionsDirty,
    permissionSkillsDirty,
    savingPermissionConfig,
    selectedAvatarAssignedSkillEntries,
    selectedAvatar,
    selectedAvatarRuntimeExtensionOptions,
    t,
    teamId,
  ]);

  useEffect(() => {
    loadAvatars(true);
  }, [loadAvatars]);

  useEffect(() => {
    try {
      const pendingAvatarId = window.localStorage.getItem(pendingManagerFocusStorageKey);
      if (!pendingAvatarId) return;
      if (!avatars.some((avatar) => avatar.id === pendingAvatarId)) return;
      setSelectedAvatarId(pendingAvatarId);
      window.localStorage.removeItem(pendingManagerFocusStorageKey);
    } catch {
      // ignore storage failure
    }
  }, [avatars, pendingManagerFocusStorageKey]);

  useEffect(() => {
    if (!selectedAvatarId) {
      setSelectedAvatar(null);
      setGovernanceEvents([]);
      setGovernanceQueue([]);
      setGovernanceQueueLoaded(false);
      setGovernance(createEmptyGovernanceState());
      setGovernanceConfig(DEFAULT_AUTOMATION_CONFIG);
      setManagerSessionId(null);
      setRuntimeLogFilter('pending');
      return;
    }
    setSelectedAvatar((current) => (current?.id === selectedAvatarId ? current : null));
    loadAvatarDetail(selectedAvatarId);
  }, [loadAvatarDetail, selectedAvatarId]);

  useEffect(() => {
    if (!selectedAvatarId) return;
    if (visibleAvatars.some((avatar) => avatar.id === selectedAvatarId)) return;
    setSelectedAvatarId(visibleAvatars[0]?.id || null);
  }, [selectedAvatarId, visibleAvatars]);

  useEffect(() => {
    if (selectedAvatarId) return;
    if (visibleAvatars.length === 0) return;
    setSelectedAvatarId(visibleAvatars[0].id);
  }, [selectedAvatarId, visibleAvatars]);

  useEffect(() => {
    scheduledGovernanceExecutionRef.current = [];
    inflightGovernanceExecutionRef.current = [];
    handledGovernanceActionIdsRef.current.clear();
    runtimeAssistantBufferRef.current = '';
  }, [managerSessionId, selectedAvatarId]);

  useEffect(() => {
    if (!selectedAvatarId) return;
    try {
      const raw = window.localStorage.getItem(`${pendingManagerComposeStoragePrefix}${selectedAvatarId}`);
      if (!raw) return;
      const parsed = JSON.parse(raw) as ChatInputComposeRequest;
      if (!parsed || typeof parsed.text !== 'string' || !parsed.text.trim()) return;
      setManagerComposeRequest({
        id: typeof parsed.id === 'string' && parsed.id.trim() ? parsed.id : makeId('timeline_prompt'),
        text: parsed.text,
        autoSend: parsed.autoSend !== false,
      });
      setTab('workspace');
      window.localStorage.removeItem(`${pendingManagerComposeStoragePrefix}${selectedAvatarId}`);
    } catch {
      // ignore malformed storage payloads
    }
  }, [pendingManagerComposeStoragePrefix, selectedAvatarId]);

  useEffect(() => {
    if (!managerProcessing) return;
    if (scheduledGovernanceExecutionRef.current.length === 0) return;
    const next = scheduledGovernanceExecutionRef.current.shift();
    if (next) {
      inflightGovernanceExecutionRef.current.push(next);
    }
  }, [managerProcessing]);

  useEffect(() => {
    const wasProcessing = previousManagerProcessingRef.current;
    previousManagerProcessingRef.current = managerProcessing;
    if (!wasProcessing || managerProcessing || !selectedAvatarId) {
      return;
    }
    loadAvatars(false).catch((error) => {
      console.error('Failed to refresh avatars after manager execution:', error);
    });
    loadAvatarDetail(selectedAvatarId).catch((error) => {
      console.error('Failed to refresh avatar detail after manager execution:', error);
    });
  }, [loadAvatarDetail, loadAvatars, managerProcessing, selectedAvatarId]);

  useEffect(() => {
    if (selectedAvatarId) return;
    if (!selectedManagerGroupId) {
      setManagerSessionId(null);
      return;
    }
    try {
      const key = `${sessionStoragePrefix}__bootstrap:${selectedManagerGroupId}`;
      const saved = window.localStorage.getItem(key);
      setManagerSessionId(saved || null);
    } catch {
      setManagerSessionId(null);
    }
  }, [selectedManagerGroupId, selectedAvatarId, sessionStoragePrefix]);

  const loadPersistedRuntimeEvents = useCallback(async (
    sessionId: string,
    options?: {
      mode?: PersistedEventLoadMode;
      silent?: boolean;
    },
  ) => {
    const mode = options?.mode || 'latest';
    const silent = options?.silent ?? false;

    if (mode === 'older') {
      setPersistedEventsLoadingMore(true);
    } else if (!silent) {
      setPersistedEventsLoading(true);
    }
    setPersistedEventsError(null);
    try {
      const query: Parameters<typeof chatApi.listSessionEvents>[1] = {
        runId: '__all__',
        limit: PERSISTED_EVENTS_PAGE_SIZE,
      };

      if (mode === 'latest') {
        query.order = 'desc';
      } else if (mode === 'older') {
        const beforeId = persistedOldestEventIdRef.current;
        if (!beforeId || beforeId <= 0) {
          setPersistedEventsHasMore(false);
          return;
        }
        query.order = 'desc';
        query.beforeEventId = beforeId;
      } else if (mode === 'incremental') {
        const afterId = persistedLatestEventIdRef.current;
        if (!afterId || afterId <= 0) {
          return;
        }
        query.order = 'asc';
        query.afterEventId = afterId;
      }

      const fetched = await chatApi.listSessionEvents(sessionId, query);
      const normalized = mode === 'latest' || mode === 'older' ? fetched.slice().reverse() : fetched;
      const base = mode === 'latest' ? [] : persistedEventsRef.current;
      const merged = mode === 'latest'
        ? normalized
        : mergePersistedEvents(base, normalized, mode);

      persistedEventsRef.current = merged;
      setPersistedEvents(merged);

      if (merged.length > 0) {
        persistedOldestEventIdRef.current = merged[0].event_id;
        persistedLatestEventIdRef.current = merged[merged.length - 1].event_id;
      } else {
        persistedOldestEventIdRef.current = null;
        persistedLatestEventIdRef.current = null;
      }

      if (mode === 'latest' || mode === 'older') {
        setPersistedEventsHasMore(fetched.length >= PERSISTED_EVENTS_PAGE_SIZE);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('common.error');
      setPersistedEventsError(msg);
      if (!silent) addToast('error', msg);
    } finally {
      if (mode === 'older') {
        setPersistedEventsLoadingMore(false);
      } else if (!silent) {
        setPersistedEventsLoading(false);
      }
    }
  }, [addToast, t]);

  useEffect(() => {
    if (!managerSessionId) {
      setPersistedEvents([]);
      persistedEventsRef.current = [];
      persistedOldestEventIdRef.current = null;
      persistedLatestEventIdRef.current = null;
      setPersistedEventsHasMore(false);
      setPersistedEventsError(null);
      return;
    }
    loadPersistedRuntimeEvents(managerSessionId, { mode: 'latest', silent: false });
  }, [loadPersistedRuntimeEvents, managerSessionId]);

  useEffect(() => {
    if (!managerProcessing || !managerSessionId) return;
    const timer = window.setInterval(() => {
      loadPersistedRuntimeEvents(managerSessionId, { mode: 'incremental', silent: true });
    }, 5000);
    return () => window.clearInterval(timer);
  }, [loadPersistedRuntimeEvents, managerProcessing, managerSessionId]);

  const persistGovernance = useCallback((
    updater: (current: AvatarGovernanceState) => AvatarGovernanceState,
    successMessage?: string,
  ) => {
    const targetAvatarId = selectedAvatarRef.current?.id;
    if (!targetAvatarId) {
      return Promise.resolve();
    }

    governancePersistQueueRef.current = governancePersistQueueRef.current
      .catch(() => undefined)
      .then(async () => {
        const avatar = selectedAvatarRef.current;
        if (!avatar || avatar.id !== targetAvatarId) return;

        const next = updater(governanceRef.current);
        governancePersistInFlightRef.current += 1;
        setSavingGovernance(true);
        try {
          const governancePayload = await avatarPortalApi.updateGovernance(teamId, avatar.id, {
            state: next as unknown as Record<string, unknown>,
          });
          const nextSettings = mergeGovernanceSettings(
            mergeGovernanceAutomationConfig(
              avatar.settings as Record<string, unknown> | null | undefined,
              readGovernanceAutomationConfig({
                digitalAvatarGovernanceConfig: governancePayload.config,
              }),
            ),
            next,
          );
          const nextAvatar = {
            ...avatar,
            settings: nextSettings,
          };
          setSelectedAvatar(nextAvatar);
          selectedAvatarRef.current = nextAvatar;
          setGovernance(next);
          governanceRef.current = next;
          setAvatarProjectionMap((current) => {
            const projection = current[targetAvatarId];
            if (!projection) return current;
            return {
              ...current,
              [targetAvatarId]: {
                ...projection,
                governanceCounts: buildGovernanceCounts(next),
                projectedAt: governancePayload.updated_at,
              },
            };
          });
          await loadGovernanceEvents(targetAvatarId);
          await loadGovernanceQueue(targetAvatarId);
          if (successMessage) addToast('success', successMessage);
        } catch (err) {
          addToast('error', err instanceof Error ? err.message : t('common.error'));
        } finally {
          governancePersistInFlightRef.current = Math.max(
            0,
            governancePersistInFlightRef.current - 1,
          );
          if (governancePersistInFlightRef.current === 0) {
            setSavingGovernance(false);
          }
        }
      });
    return governancePersistQueueRef.current;
  }, [addToast, loadGovernanceEvents, loadGovernanceQueue, t, teamId]);

  const createManagerSession = useCallback(async (): Promise<string> => {
    if (selectedAvatarDisplay) {
      const managerAgentId =
        selectedAvatarDisplay.codingAgentId ||
        selectedAvatarDisplay.agentId ||
        selectedAvatarDisplay.serviceAgentId ||
        undefined;
      const res = await chatApi.createPortalManagerSession(teamId, managerAgentId, selectedAvatarDisplay.id);
      try {
        window.localStorage.setItem(`${sessionStoragePrefix}${selectedAvatarDisplay.id}`, res.session_id);
      } catch {}
      setManagerSessionId(res.session_id);
      return res.session_id;
    }
    const managerId = selectedManagerGroupId;
    if (!managerId) {
      throw new Error(t('digitalAvatar.states.noManagerAgent'));
    }
    const res = await chatApi.createPortalManagerSession(teamId, managerId);
    try {
      window.localStorage.setItem(`${sessionStoragePrefix}__bootstrap:${managerId}`, res.session_id);
    } catch {}
    setManagerSessionId(res.session_id);
    return res.session_id;
  }, [selectedAvatarDisplay, selectedManagerGroupId, sessionStoragePrefix, t, teamId]);

  const onManagerSessionCreated = useCallback((sessionId: string) => {
    setManagerSessionId(sessionId);
    const key = selectedAvatarDisplay
      ? `${sessionStoragePrefix}${selectedAvatarDisplay.id}`
      : (selectedManagerGroupId
        ? `${sessionStoragePrefix}__bootstrap:${selectedManagerGroupId}`
        : null);
    if (!key) return;
    try {
      window.localStorage.setItem(key, sessionId);
    } catch {}
  }, [selectedAvatarDisplay, selectedManagerGroupId, sessionStoragePrefix]);

  const sendManagerQuickPrompt = useCallback((kind:
    | 'createExternalSupport'
    | 'createExternalPartner'
    | 'createInternalKnowledge'
    | 'createInternalOps'
    | 'audit'
    | 'optimize'
    | 'releaseChecklist'
    | 'optimizePrompt'
    | 'setAggressive'
    | 'setBalanced'
    | 'setConservative'
  ) => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const currentName = selectedAvatar?.name || t('digitalAvatar.workspace.currentAvatarFallback', '当前分身');
    const currentPortalId = selectedAvatar?.id || '';
    let text = '';
    if (kind === 'createExternalSupport') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateExternalSupport',
        '请为我创建一个新的对外数字分身，定位为客服答疑/售后分流助手：先明确服务对象、知识边界和升级到人工的规则，再调用 create_digital_avatar 创建，并回读 profile 做校验，最后给我结果与风险清单。'
      )}\n${t(
        'digitalAvatar.workspace.quickPromptManagerPinned',
        '管理 Agent 固定为 {{managerId}}，新分身必须继续归属这个管理组。',
        { managerId: effectiveManagerAgentId }
      )}`;
    } else if (kind === 'createExternalPartner') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateExternalPartner',
        '请为我创建一个新的对外数字分身，定位为客户/合作伙伴协同入口：先梳理可开放的文档范围、协作边界和常见问题，再调用 create_digital_avatar 创建，并回读 profile 校验，最后给我发布建议。'
      )}\n${t(
        'digitalAvatar.workspace.quickPromptManagerPinned',
        '管理 Agent 固定为 {{managerId}}，新分身必须继续归属这个管理组。',
        { managerId: effectiveManagerAgentId }
      )}`;
    } else if (kind === 'createInternalKnowledge') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateInternalKnowledge',
        '请为我创建一个新的对内数字分身，定位为知识问答/制度检索助手：先定义内部使用场景、文档边界和回答风格，再调用 create_digital_avatar 创建，并回读 profile 校验，最后输出上线建议。'
      )}\n${t(
        'digitalAvatar.workspace.quickPromptManagerPinned',
        '管理 Agent 固定为 {{managerId}}，新分身必须继续归属这个管理组。',
        { managerId: effectiveManagerAgentId }
      )}`;
    } else if (kind === 'createInternalOps') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateInternalOps',
        '请为我创建一个新的对内数字分身，定位为流程执行/任务跟进助手：先定义任务目标、触发方式和审批边界，再调用 create_digital_avatar 创建，并回读 profile 校验，最后输出执行建议。'
      )}\n${t(
        'digitalAvatar.workspace.quickPromptManagerPinned',
        '管理 Agent 固定为 {{managerId}}，新分身必须继续归属这个管理组。',
        { managerId: effectiveManagerAgentId }
      )}`;
    } else if (kind === 'audit') {
      text = t(
        'digitalAvatar.workspace.quickPromptAudit',
        '请检查“{{name}}”当前能力边界（文档权限、扩展、技能、提示词），给出三项最小改进建议并标注风险。',
        { name: currentName }
      );
    } else if (kind === 'optimize') {
      if (!selectedAvatar || !currentPortalId) {
        addToast('error', t('digitalAvatar.workspace.quickNeedAvatar', '请先在左侧选择一个分身，再设置治理阈值档位。'));
        return;
      }
      text = t(
        'digitalAvatar.workspace.quickPromptOptimize',
        '请基于“{{name}}”最近运行情况，产出一份可执行优化工单（问题证据、修复方案、验证标准、回滚条件）。',
        { name: currentName }
      );
    } else if (kind === 'releaseChecklist') {
      if (!selectedAvatar || !currentPortalId) {
        addToast('error', t('digitalAvatar.workspace.quickNeedAvatar', '请先在左侧选择一个分身，再设置治理阈值档位。'));
        return;
      }
      text = t(
        'digitalAvatar.workspace.quickPromptReleaseChecklist',
        '请为“{{name}}”生成一份发布前检查清单，重点覆盖文档权限、提示词、扩展/技能、升级到人工机制、访客视角说明和回滚方案，并标注必须人工确认的项。',
        { name: currentName }
      );
    } else if (kind === 'optimizePrompt') {
      if (!selectedAvatar || !currentPortalId) {
        addToast('error', t('digitalAvatar.workspace.quickNeedAvatar', '请先在左侧选择一个分身，再设置治理阈值档位。'));
        return;
      }
      text = t(
        'digitalAvatar.workspace.quickPromptOptimizePrompt',
        '请针对“{{name}}”输出一份最小可执行的优化方案，重点是提示词、技能组合、工具边界和回答风格，并按低/中/高风险分层说明。',
        { name: currentName }
      );
    } else {
      if (!selectedAvatar || !currentPortalId) {
        addToast('error', t('digitalAvatar.workspace.quickNeedAvatar', '请先在左侧选择一个分身，再设置治理阈值档位。'));
        return;
      }
      const threshold =
        kind === 'setAggressive' ? 3 : kind === 'setBalanced' ? 5 : 7;
      text = t(
        'digitalAvatar.workspace.quickPromptSetThreshold',
        '请把分身“{{name}}”(portal_id={{portalId}}) 的自动治理阈值设置为 {{count}}。请调用 portal_tools__configure_portal_service_agent 并通过 settings_patch 写入 digitalAvatarGovernanceConfig.autoProposalTriggerCount={{count}}，然后调用 portal_tools__get_portal_service_capability_profile 回读校验并汇报结果与风险。',
        {
          name: currentName,
          portalId: currentPortalId,
          count: threshold,
        }
      );
    }
    setManagerComposeRequest({
      id: makeId('quick_prompt'),
      text,
      autoSend: false,
    });
    addToast('success', t('digitalAvatar.actions.quickPromptPrepared', '已填入输入框'));
  }, [addToast, effectiveManagerAgentId, selectedAvatar, t]);

  const managerQuickActionGroups = useMemo<ChatInputQuickActionGroup[]>(() => {
    const groups: ChatInputQuickActionGroup[] = [
      {
        key: 'create',
        label: t('digitalAvatar.workspace.quickGroupCreate', '创建模板'),
        actions: [
          {
            key: 'createExternalSupport',
            label: t('digitalAvatar.workspace.quickCreateExternalSupport', '新建对外客服分身'),
            description: t('digitalAvatar.workspace.quickCreateExternalSupportDesc', '适合客服答疑、售后分流和常见问题处理。'),
            onSelect: () => sendManagerQuickPrompt('createExternalSupport'),
          },
          {
            key: 'createExternalPartner',
            label: t('digitalAvatar.workspace.quickCreateExternalPartner', '新建对外协作分身'),
            description: t('digitalAvatar.workspace.quickCreateExternalPartnerDesc', '适合客户、合作伙伴或供应商协同入口。'),
            onSelect: () => sendManagerQuickPrompt('createExternalPartner'),
          },
          {
            key: 'createInternalKnowledge',
            label: t('digitalAvatar.workspace.quickCreateInternalKnowledge', '新建对内知识助手'),
            description: t('digitalAvatar.workspace.quickCreateInternalKnowledgeDesc', '适合制度检索、知识问答和文档导航。'),
            onSelect: () => sendManagerQuickPrompt('createInternalKnowledge'),
          },
          {
            key: 'createInternalOps',
            label: t('digitalAvatar.workspace.quickCreateInternalOps', '新建对内执行助手'),
            description: t('digitalAvatar.workspace.quickCreateInternalOpsDesc', '适合流程执行、任务跟进和内部操作协同。'),
            onSelect: () => sendManagerQuickPrompt('createInternalOps'),
          },
        ],
      },
    ];

    if (selectedAvatar) {
      groups.push({
        key: 'govern',
        label: t('digitalAvatar.workspace.quickGroupGovern', '治理检查'),
        actions: [
          {
            key: 'audit',
            label: t('digitalAvatar.workspace.quickAuditCurrent', '审查当前能力'),
            description: t('digitalAvatar.workspace.quickAuditCurrentDesc', '检查文档权限、扩展、技能和提示词边界。'),
            onSelect: () => sendManagerQuickPrompt('audit'),
          },
          {
            key: 'releaseChecklist',
            label: t('digitalAvatar.workspace.quickReleaseChecklist', '生成发布前检查清单'),
            description: t('digitalAvatar.workspace.quickReleaseChecklistDesc', '整理上线前必须确认的权限、说明和回滚项。'),
            onSelect: () => sendManagerQuickPrompt('releaseChecklist'),
          },
          {
            key: 'optimize',
            label: t('digitalAvatar.workspace.quickOptimizeCurrent', '生成优化工单'),
            description: t('digitalAvatar.workspace.quickOptimizeCurrentDesc', '基于最近运行情况，产出可执行优化工单。'),
            onSelect: () => sendManagerQuickPrompt('optimize'),
          },
          {
            key: 'optimizePrompt',
            label: t('digitalAvatar.workspace.quickOptimizePrompt', '优化提示词与技能'),
            description: t('digitalAvatar.workspace.quickOptimizePromptDesc', '聚焦提示词、技能组合和回答风格的最小优化。'),
            onSelect: () => sendManagerQuickPrompt('optimizePrompt'),
          },
        ],
      });
    }

    if (recommendedQuickActions.length > 0) {
      groups.push({
        key: 'policy',
        label: t('digitalAvatar.workspace.quickGroupPolicy', '策略预设'),
        actions: recommendedQuickActions.map((action) => ({
          key: action.key,
          label: action.label,
          description:
            action.kind === 'setAggressive'
              ? t('digitalAvatar.workspace.quickSetAggressiveDesc', '更积极地产生治理提案，适合内部执行型分身。')
              : action.kind === 'setConservative'
              ? t('digitalAvatar.workspace.quickSetConservativeDesc', '更谨慎地产生治理提案，适合对外交付型分身。')
              : action.kind === 'setBalanced'
              ? t('digitalAvatar.workspace.quickSetBalancedDesc', '在自动治理频率和稳定性之间保持平衡。')
              : t('digitalAvatar.workspace.quickOptimizeCurrentDesc', '基于最近运行情况，产出可执行优化工单。'),
          onSelect: () => sendManagerQuickPrompt(action.kind),
        })),
      });
    }

    return groups;
  }, [recommendedQuickActions, sendManagerQuickPrompt, t]);

  const copyGuideCommand = useCallback(async (text: string) => {
    if (await copyText(text)) {
      addToast('success', t('digitalAvatar.actions.copiedPrompt', '已复制'));
      return;
    }
    addToast('error', t('common.error'));
  }, [addToast, t]);

  const sendGuideCommandToManager = useCallback((text: string) => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    setManagerComposeRequest({
      id: makeId('guide_prompt'),
      text,
      autoSend: true,
    });
    setTab('workspace');
    addToast('success', t('digitalAvatar.actions.quickPromptSent', '已发送'));
  }, [addToast, effectiveManagerAgentId, t]);

  const dispatchGovernanceExecution = useCallback((
    text: string,
    binding?: Omit<GovernanceExecutionBinding, 'id'>,
  ) => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const prompt = text.trim();
    if (!prompt) return;
    const hasPendingExecution =
      managerProcessing ||
      inflightGovernanceExecutionRef.current.length > 0 ||
      scheduledGovernanceExecutionRef.current.length > 0;
    if (hasPendingExecution) {
      addToast(
        'warning',
        t(
          'digitalAvatar.governance.executionBusy',
          '管理 Agent 正在执行上一条治理任务，请等待完成后再提交下一项。'
        )
      );
      return;
    }
    const requestId = makeId('governance_exec');
    handledGovernanceActionIdsRef.current.delete(requestId);
    const structuredPrompt = `${prompt}\n\n${t(
      'digitalAvatar.governance.structuredReceiptInstruction',
      '请在最终回复末尾严格追加下面这个结构化回执（不要解释，也不要省略）：'
    )}\n<governance_action_result>{\"action_id\":\"${requestId}\",\"outcome\":\"success|partial|failed\",\"summary\":\"一句话结果摘要\",\"reason\":\"失败/部分成功原因；成功可留空\"}</governance_action_result>`;
    setManagerComposeRequest({
      id: requestId,
      text: structuredPrompt,
      autoSend: true,
    });
    if (binding) {
      scheduledGovernanceExecutionRef.current.push({
        ...binding,
        id: requestId,
      });
    }
    addToast('success', t('digitalAvatar.governance.executionQueued', '已提交管理 Agent 执行'));
  }, [addToast, effectiveManagerAgentId, managerProcessing, t]);

  const applyGovernanceBindingOutcome = useCallback((
    binding: GovernanceExecutionBinding,
    outcome: GovernanceActionReceipt['outcome'],
    reasonText?: string,
  ) => {
    const now = toIsoNow();
    const failed = outcome === 'failed';
    const partial = outcome === 'partial';
    const reason = (reasonText || '').trim();
    persistGovernance((current) => {
      if (binding.entityType === 'capability') {
        return {
          ...current,
          capabilityRequests: current.capabilityRequests.map((item) => {
            if (item.id !== binding.targetId) return item;
            if (!failed && !partial) {
              return {
                ...item,
                status: binding.targetStatus as CapabilityGapRequest['status'],
                decisionReason: reason || item.decisionReason,
                updatedAt: now,
              };
            }
            return {
              ...item,
              status: 'needs_human',
              decision: 'require_human_confirm',
              decisionReason: reason || item.decisionReason,
              updatedAt: now,
            };
          }),
        };
      }
      if (binding.entityType === 'proposal') {
        return {
          ...current,
          gapProposals: current.gapProposals.map((item) =>
            item.id !== binding.targetId
              ? item
              : {
                  ...item,
                  status: failed || partial ? 'pending_approval' : (binding.targetStatus as ProposalStatus),
                  updatedAt: now,
                }
          ),
        };
      }
      return {
        ...current,
        optimizationTickets: current.optimizationTickets.map((item) => {
          if (item.id !== binding.targetId) return item;
          if (!failed && !partial) {
            return {
              ...item,
              status: binding.targetStatus as OptimizationStatus,
              updatedAt: now,
            };
          }
          return {
            ...item,
            status: binding.targetStatus === 'deployed' ? 'rolled_back' : 'rejected',
            updatedAt: now,
          };
        }),
      };
    });
  }, [persistGovernance]);

  const takeGovernanceBindingByActionId = useCallback((actionId: string): GovernanceExecutionBinding | undefined => {
    const normalized = actionId.trim();
    if (!normalized) return undefined;
    const inflightIdx = inflightGovernanceExecutionRef.current.findIndex((item) => item.id === normalized);
    if (inflightIdx >= 0) {
      const [picked] = inflightGovernanceExecutionRef.current.splice(inflightIdx, 1);
      return picked;
    }
    const scheduledIdx = scheduledGovernanceExecutionRef.current.findIndex((item) => item.id === normalized);
    if (scheduledIdx >= 0) {
      const [picked] = scheduledGovernanceExecutionRef.current.splice(scheduledIdx, 1);
      return picked;
    }
    return undefined;
  }, []);

  const applyGovernanceReceipt = useCallback((receipt: GovernanceActionReceipt): boolean => {
    const actionId = receipt.actionId.trim();
    if (!actionId) return false;
    if (handledGovernanceActionIdsRef.current.has(actionId)) return true;
    const binding = takeGovernanceBindingByActionId(actionId);
    if (!binding) return false;

    handledGovernanceActionIdsRef.current.add(actionId);
    applyGovernanceBindingOutcome(binding, receipt.outcome, receipt.reason || receipt.summary);
    return true;
  }, [applyGovernanceBindingOutcome, takeGovernanceBindingByActionId]);

  const createAutoCapabilityRequest = useCallback((item: RuntimeSuggestion, nowIso: string): CapabilityGapRequest => ({
    id: makeId('gap'),
    title: item.title.trim(),
    detail: item.evidence.trim(),
    requestedScope: [`problem:${item.problemType}`],
    risk: item.risk,
    status: 'pending',
    source: 'avatar',
    createdAt: nowIso,
    updatedAt: nowIso,
  }), []);

  const createAutoOptimizationTicket = useCallback((item: RuntimeSuggestion, nowIso: string): OptimizationTicket => ({
    id: makeId('opt'),
    title: item.title.trim(),
    problemType: item.problemType,
    evidence: item.evidence.trim(),
    proposal: item.proposal.trim(),
    expectedGain: item.expectedGain.trim(),
    risk: item.risk,
    status: 'pending',
    createdAt: nowIso,
    updatedAt: nowIso,
  }), []);

  const maybeCreateAutoGapProposal = useCallback((
    current: AvatarGovernanceState,
    item: RuntimeSuggestion,
    nowIso: string,
  ): AgentGapProposal | null => {
    const scopeToken = `problem:${item.problemType}`;
    const relatedPendingRequestCount = current.capabilityRequests.filter(
      (request) =>
        request.status === 'pending' &&
        request.requestedScope.some((scope) => scope === scopeToken),
    ).length;
    if (relatedPendingRequestCount < governanceConfig.autoProposalTriggerCount) {
      return null;
    }

    const proposalTitle = t(
      'digitalAvatar.governance.autoProposalTitle',
      '新增分身提案：{{problem}}能力长期缺口',
      { problem: item.problemType },
    );
    const hasOpenProposal = current.gapProposals.some(
      (proposal) =>
        proposal.title === proposalTitle &&
        proposal.status !== 'rejected',
    );
    if (hasOpenProposal) {
      return null;
    }

    return {
      id: makeId('proposal'),
      title: proposalTitle,
      description: t(
        'digitalAvatar.governance.autoProposalDescription',
        '近阶段该类型能力缺口多次重复出现，建议新增专用分身并进入人工审批。',
      ),
      expectedGain: t(
        'digitalAvatar.governance.autoProposalGain',
        '通过能力隔离减少重复提权与失败重试，提升交付稳定性。',
      ),
      status: 'pending_approval',
      proposedBy: 'manager',
      createdAt: nowIso,
      updatedAt: nowIso,
    };
  }, [governanceConfig.autoProposalTriggerCount, t]);

  const handleRuntimeEvent = useCallback((event: ChatRuntimeEvent) => {
    if (event.kind === 'text') {
      const chunk = event.text || '';
      if (!chunk) return;
      const merged = `${runtimeAssistantBufferRef.current}${chunk}`;
      runtimeAssistantBufferRef.current = merged.length > 120000
        ? merged.slice(-120000)
        : merged;
      return;
    }

    if (event.kind === 'done') {
      const receipts = parseGovernanceActionReceipts(runtimeAssistantBufferRef.current);
      runtimeAssistantBufferRef.current = '';
      if (receipts.length > 0) {
        for (const receipt of receipts) {
          applyGovernanceReceipt(receipt);
        }
      } else {
        let binding: GovernanceExecutionBinding | undefined;
        while (inflightGovernanceExecutionRef.current.length > 0) {
          const candidate = inflightGovernanceExecutionRef.current.shift();
          if (!candidate) break;
          if (handledGovernanceActionIdsRef.current.has(candidate.id)) continue;
          binding = candidate;
          break;
        }
        if (binding) {
          handledGovernanceActionIdsRef.current.add(binding.id);
          const failed = isRuntimeDoneFailure(event.detail);
          const outcome: GovernanceActionReceipt['outcome'] = failed ? 'failed' : 'success';
          const reason = typeof event.detail?.error === 'string' ? event.detail.error : undefined;
          applyGovernanceBindingOutcome(binding, outcome, reason);
        }
      }
    }

    const suggestion = summarizeRuntimeFailure(event, runtimeSuggestionText);
    if (!suggestion) return;

    persistGovernance((current) => {
      const duplicateRuntime = current.runtimeLogs.some(
        (item) =>
          item.title === suggestion.title &&
          item.evidence === suggestion.evidence,
      );
      if (duplicateRuntime) return current;

      const nowIso = toIsoNow();
      const hasSamePendingRequest = current.capabilityRequests.some(
        (request) =>
          request.status === 'pending' &&
          request.title === suggestion.title &&
          request.detail === suggestion.evidence,
      );
      const hasSamePendingTicket = current.optimizationTickets.some(
        (ticket) =>
          ticket.status === 'pending' &&
          ticket.title === suggestion.title &&
          ticket.problemType === suggestion.problemType &&
          ticket.evidence === suggestion.evidence,
      );

      let capabilityRequests = current.capabilityRequests;
      let optimizationTickets = current.optimizationTickets;
      let gapProposals = current.gapProposals;
      let runtimeStatus: RuntimeLogStatus = 'pending';

      if (!hasSamePendingRequest) {
        capabilityRequests = [createAutoCapabilityRequest(suggestion, nowIso), ...capabilityRequests];
        runtimeStatus = 'requested';
      }
      if (!hasSamePendingTicket) {
        optimizationTickets = [createAutoOptimizationTicket(suggestion, nowIso), ...optimizationTickets];
        if (runtimeStatus === 'pending') runtimeStatus = 'ticketed';
      }

      const autoProposal = maybeCreateAutoGapProposal(
        { ...current, capabilityRequests, optimizationTickets, gapProposals },
        suggestion,
        nowIso,
      );
      if (autoProposal) {
        gapProposals = [autoProposal, ...gapProposals];
      }

      return {
        ...current,
        capabilityRequests,
        optimizationTickets,
        gapProposals,
        runtimeLogs: [{ ...suggestion, status: runtimeStatus }, ...current.runtimeLogs]
          .slice(0, MAX_RUNTIME_LOGS),
      };
    });
  }, [
    applyGovernanceBindingOutcome,
    applyGovernanceReceipt,
    createAutoCapabilityRequest,
    createAutoOptimizationTicket,
    maybeCreateAutoGapProposal,
    persistGovernance,
    runtimeSuggestionText,
  ]);

  const updateRuntimeSuggestionStatus = useCallback((id: string, status: RuntimeLogStatus) => {
    persistGovernance((current) => ({
      ...current,
      runtimeLogs: current.runtimeLogs.map((item) => (item.id === id ? { ...item, status } : item)),
    }));
  }, [persistGovernance]);

  const dismissRuntimeSuggestion = useCallback((id: string) => {
    updateRuntimeSuggestionStatus(id, 'dismissed');
  }, [updateRuntimeSuggestionStatus]);

  const resetRuntimeSuggestion = useCallback((id: string) => {
    updateRuntimeSuggestionStatus(id, 'pending');
  }, [updateRuntimeSuggestionStatus]);

  const decideCapabilityRequest = useCallback((id: string, decision: DecisionMode, reason?: string) => {
    const now = toIsoNow();
    const item = governanceRef.current.capabilityRequests.find((it) => it.id === id);
    const portalId = selectedAvatarRef.current?.id;
    persistGovernance(
      current => ({
        ...current,
        capabilityRequests: current.capabilityRequests.map((item) =>
          item.id !== id
            ? item
            : {
                ...item,
                status: toDecisionStatus(decision),
                decision,
                decisionReason: reason || item.decisionReason,
                updatedAt: now,
              }
        ),
      }),
      t('common.saved')
    );
    if (
      item &&
      portalId &&
      (decision === 'approve_direct' || decision === 'approve_sandbox')
    ) {
      const mode = decision === 'approve_sandbox' ? 'sandbox' : 'direct';
      const executionPrompt = t(
        'digitalAvatar.governance.capabilityExecutionPrompt',
        '请执行能力缺口请求并完成回读校验。portal_id={{portalId}}，模式={{mode}}。\n请求标题：{{title}}\n请求说明：{{detail}}\n要求：\n1) 优先调用 portal_tools__configure_portal_service_agent 完成最小权限变更；\n2) 必须调用 portal_tools__get_portal_service_capability_profile 回读验证；\n3) 输出变更摘要、风险与回滚建议。',
        {
          portalId,
          mode,
          title: item.title,
          detail: item.detail,
        }
      );
      dispatchGovernanceExecution(executionPrompt, {
        entityType: 'capability',
        targetId: item.id,
        targetStatus: toDecisionStatus(decision),
      });
    }
  }, [dispatchGovernanceExecution, persistGovernance, t]);

  const updateGapProposalStatus = useCallback((id: string, status: ProposalStatus) => {
    const now = toIsoNow();
    const item = governanceRef.current.gapProposals.find((it) => it.id === id);
    persistGovernance(
      current => ({
        ...current,
        gapProposals: current.gapProposals.map((item) =>
          item.id === id ? { ...item, status, updatedAt: now } : item
        ),
      }),
      t('common.saved')
    );
    if (item && ['approved', 'pilot', 'active'].includes(status)) {
      const executionPrompt = t(
        'digitalAvatar.governance.proposalExecutionPrompt',
        '请根据已通过提案进入执行闭环：\n提案：{{title}}\n说明：{{desc}}\n目标状态：{{status}}\n要求：产出执行计划（能力/权限/文档范围）、实施步骤、验证标准与回滚策略。',
        {
          title: item.title,
          desc: item.description,
          status,
        }
      );
      dispatchGovernanceExecution(executionPrompt, {
        entityType: 'proposal',
        targetId: item.id,
        targetStatus: status,
      });
    }
  }, [dispatchGovernanceExecution, persistGovernance, t]);

  const updateOptimizationStatus = useCallback((id: string, status: OptimizationStatus) => {
    const now = toIsoNow();
    const item = governanceRef.current.optimizationTickets.find((it) => it.id === id);
    persistGovernance(
      current => ({
        ...current,
        optimizationTickets: current.optimizationTickets.map((item) =>
          item.id === id ? { ...item, status, updatedAt: now } : item
        ),
      }),
      t('common.saved')
    );
    if (item && ['approved', 'experimenting', 'deployed'].includes(status)) {
      const executionPrompt = t(
        'digitalAvatar.governance.ticketExecutionPrompt',
        '请执行优化工单并回传结果：\n工单：{{title}}\n问题类型：{{problemType}}\n证据：{{evidence}}\n方案：{{proposal}}\n目标状态：{{status}}\n要求：执行后提供验证结果、风险变化与是否继续推进建议。',
        {
          title: item.title,
          problemType: item.problemType,
          evidence: item.evidence,
          proposal: item.proposal,
          status,
        }
      );
      dispatchGovernanceExecution(executionPrompt, {
        entityType: 'ticket',
        targetId: item.id,
        targetStatus: status,
      });
    }
  }, [dispatchGovernanceExecution, persistGovernance, t]);

  const clearRuntimeSuggestions = () => {
    persistGovernance((current) => ({ ...current, runtimeLogs: [] }));
  };

  const refreshPersistedEvents = useCallback(() => {
    if (!managerSessionId) {
      addToast('error', t('digitalAvatar.governance.runtimeEventsNoSession', '暂无可追溯会话，请先与管理 Agent 开始对话。'));
      return;
    }
    loadPersistedRuntimeEvents(managerSessionId, { mode: 'latest', silent: false });
  }, [addToast, loadPersistedRuntimeEvents, managerSessionId, t]);

  const loadOlderPersistedEvents = useCallback(() => {
    if (!managerSessionId) {
      addToast('error', t('digitalAvatar.governance.runtimeEventsNoSession', '暂无可追溯会话，请先与管理 Agent 开始对话。'));
      return;
    }
    if (!persistedEventsHasMore || persistedEventsLoadingMore) return;
    loadPersistedRuntimeEvents(managerSessionId, { mode: 'older', silent: false });
  }, [
    addToast,
    loadPersistedRuntimeEvents,
    managerSessionId,
    persistedEventsHasMore,
    persistedEventsLoadingMore,
    t,
  ]);

  const saveAutomationConfig = useCallback(async () => {
    if (!selectedAvatar || !canManage) return;
    const nextValue = Math.min(10, Math.max(1, Math.round(autoProposalTriggerCountDraft || 3)));
    const nextDraftConfig = {
      ...governanceConfig,
      autoProposalTriggerCount: nextValue,
    };
    setSavingAutomationConfig(true);
    try {
      const governancePayload = await avatarPortalApi.updateGovernance(teamId, selectedAvatar.id, {
        config: nextDraftConfig,
      });
      const nextConfig = readGovernanceAutomationConfig({
        digitalAvatarGovernanceConfig: governancePayload.config,
      });
      const nextSettings = mergeGovernanceAutomationConfig(
        selectedAvatar.settings as Record<string, unknown> | null | undefined,
        nextConfig,
      );
      const nextAvatar = {
        ...selectedAvatar,
        settings: nextSettings,
      };
      setSelectedAvatar(nextAvatar);
      selectedAvatarRef.current = nextAvatar;
      setGovernanceConfig(nextConfig);
      setAutoProposalTriggerCountDraft(nextValue);
      await loadGovernanceEvents(selectedAvatar.id);
      await loadGovernanceQueue(selectedAvatar.id);
      addToast('success', t('common.saved'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSavingAutomationConfig(false);
    }
  }, [
    addToast,
    autoProposalTriggerCountDraft,
    canManage,
    governanceConfig,
    loadGovernanceEvents,
    loadGovernanceQueue,
    selectedAvatar,
    t,
    teamId,
  ]);

  const openEcosystem = () => {
      navigate(`/admin/teams/${teamId}?section=ecosystem`);
  };

  const handleToggleAvatarPublish = useCallback(async () => {
    if (!selectedAvatar || publishingAvatar) return;
    setPublishingAvatar(true);
    try {
      const detail = selectedAvatarStatus === 'published'
        ? await avatarPortalApi.unpublish(teamId, selectedAvatar.id)
        : await avatarPortalApi.publish(teamId, selectedAvatar.id);
      setSelectedAvatar(detail);
      selectedAvatarRef.current = detail;
      await loadAvatars(false);
      addToast(
        'success',
        selectedAvatarStatus === 'published'
          ? t('digitalAvatar.publish.unpublishSuccess', '已停止对外服务，当前回到草稿状态')
          : t('digitalAvatar.publish.publishSuccess', '已发布分身，访客可通过正式入口访问'),
      );
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setPublishingAvatar(false);
    }
  }, [addToast, loadAvatars, publishingAvatar, selectedAvatar, selectedAvatarStatus, t, teamId]);

  const handleToggleChatWidget = useCallback(async () => {
    if (!selectedAvatar || !canManage || updatingPublicConfig) return;
    setUpdatingPublicConfig(true);
    try {
      const currentSettings = (selectedAvatar.settings as Record<string, unknown> | undefined) || {};
      const detail = await avatarPortalApi.update(teamId, selectedAvatar.id, {
        settings: {
          ...currentSettings,
          showChatWidget: !selectedAvatarShowChatWidget,
        },
      });
      setSelectedAvatar(detail);
      selectedAvatarRef.current = detail;
      await loadAvatars(false);
      addToast(
        'success',
        !selectedAvatarShowChatWidget
          ? t('digitalAvatar.workspace.chatWidgetEnabled', '已开启默认聊天挂件')
          : t('digitalAvatar.workspace.chatWidgetDisabled', '已关闭默认聊天挂件'),
      );
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setUpdatingPublicConfig(false);
    }
  }, [
    addToast,
    canManage,
    loadAvatars,
    selectedAvatar,
    selectedAvatarShowChatWidget,
    t,
    teamId,
    updatingPublicConfig,
  ]);

  const handleToggleBoundDocuments = useCallback(async () => {
    if (!selectedAvatar || !canManage || updatingPublicConfig) return;
    setUpdatingPublicConfig(true);
    try {
      const currentSettings = (selectedAvatar.settings as Record<string, unknown> | undefined) || {};
      const detail = await avatarPortalApi.update(teamId, selectedAvatar.id, {
        settings: {
          ...currentSettings,
          showBoundDocuments: !selectedAvatarShowBoundDocuments,
        },
      });
      setSelectedAvatar(detail);
      selectedAvatarRef.current = detail;
      await loadAvatars(false);
      addToast(
        'success',
        !selectedAvatarShowBoundDocuments
          ? t('digitalAvatar.workspace.boundDocumentsVisible', '已恢复对外展示平台资料')
          : t('digitalAvatar.workspace.boundDocumentsHidden', '已关闭对外平台资料展示'),
      );
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setUpdatingPublicConfig(false);
    }
  }, [
    addToast,
    canManage,
    loadAvatars,
    selectedAvatar,
    selectedAvatarShowBoundDocuments,
    t,
    teamId,
    updatingPublicConfig,
  ]);

  const handleSavePublicNarrative = useCallback(async () => {
    if (!selectedAvatar || !canManage || savingNarrativeConfig) return;
    setSavingNarrativeConfig(true);
    try {
      const currentSettings = { ...(((selectedAvatar.settings as Record<string, unknown> | undefined) || {})) };
      const avatarPublicNarrative = buildAvatarPublicNarrativePayload({
        heroIntro: publishHeroIntro,
        heroUseCases: splitNarrativeUseCases(publishHeroUseCasesText),
        heroWorkingStyle: publishHeroWorkingStyle,
        heroCtaHint: publishHeroCtaHint,
      });
      if (avatarPublicNarrative) {
        currentSettings.avatarPublicNarrative = avatarPublicNarrative;
      } else {
        delete currentSettings.avatarPublicNarrative;
      }
      const detail = await avatarPortalApi.update(teamId, selectedAvatar.id, {
        settings: currentSettings,
      });
      setSelectedAvatar(detail);
      selectedAvatarRef.current = detail;
      await loadAvatars(false);
      setPublicNarrativeDialogOpen(false);
      addToast('success', t('common.saved', '已保存'));
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSavingNarrativeConfig(false);
    }
  }, [
    addToast,
    canManage,
    loadAvatars,
    publishHeroCtaHint,
    publishHeroIntro,
    publishHeroUseCasesText,
    publishHeroWorkingStyle,
    savingNarrativeConfig,
    selectedAvatar,
    t,
    teamId,
  ]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    const saved = window.localStorage.getItem(`${WORKSPACE_CHROME_STORAGE_PREFIX}${teamId}`);
    if (saved !== null) {
      setWorkspaceChromeCollapsed(saved === '1');
    }
  }, [teamId]);

  useEffect(() => {
    if (typeof window === 'undefined') return;
    window.localStorage.setItem(
      `${WORKSPACE_CHROME_STORAGE_PREFIX}${teamId}`,
      workspaceChromeCollapsed ? '1' : '0',
    );
    window.localStorage.setItem(
      `${WORKSPACE_CHROME_STORAGE_PREFIX}default`,
      workspaceChromeCollapsed ? '1' : '0',
    );
  }, [teamId, workspaceChromeCollapsed]);

  const governanceStats = useMemo(() => {
    const pendingCapability = governance.capabilityRequests.filter(x => x.status === 'pending').length;
    const pendingProposals = governance.gapProposals.filter(x => x.status === 'pending_approval').length;
    const pendingTickets = governance.optimizationTickets.filter(x => x.status === 'pending').length;
    return { pendingCapability, pendingProposals, pendingTickets };
  }, [governance]);
  const mobileWorkspacePendingCount = useMemo(() => {
    if (selectedAvatarDisplay) {
      return getAvatarProjectionPendingCount(avatarProjectionMap[selectedAvatarDisplay.id]);
    }
    return governanceStats.pendingCapability + governanceStats.pendingProposals + governanceStats.pendingTickets;
  }, [
    avatarProjectionMap,
    governanceStats.pendingCapability,
    governanceStats.pendingProposals,
    governanceStats.pendingTickets,
    selectedAvatarDisplay,
  ]);
  const mobileWorkspaceContextTitle = selectedAvatarDisplay
    ? selectedAvatarDisplay.name
    : t('digitalAvatar.workspace.groupModeShort', '管理组全局模式');
  const mobileWorkspaceContextDescription = selectedAvatarDisplay
    ? (mobileWorkspacePendingCount > 0
        ? t('digitalAvatar.workspace.mobilePendingHint', '当前待处理 {{count}} 项，优先继续治理与审计。', {
            count: mobileWorkspacePendingCount,
          })
        : t('digitalAvatar.workspace.mobileContextHintCompact', '继续通过管理 Agent 协调当前分身的治理、发布和能力调整。'))
    : t('digitalAvatar.workspace.groupModeHintCompact', '聚焦管理组、待处理事项和新分身规划。');
  const mobileWorkspaceContextStatus = selectedAvatarDisplay ? selectedAvatarStatusLabel : t('digitalAvatar.workspace.groupModeBadge', '全局');

  const toggleInspectorPanel = useCallback(() => {
    setFocusMode(false);
    if (isConversationCompactLayout) {
      setActiveMobilePanel((prev) => (prev === 'console' ? null : 'console'));
      return;
    }
    setInspectorOpen((prev) => !prev);
  }, [isConversationCompactLayout]);

  const openInspectorPanel = useCallback((nextTab?: InspectorTab) => {
    if (nextTab) setInspectorTab(nextTab);
    setFocusMode(false);
    if (isConversationCompactLayout) {
      setActiveMobilePanel('console');
      return;
    }
    setInspectorOpen(true);
  }, [isConversationCompactLayout]);

  const toggleFocusMode = useCallback(() => {
    setFocusMode((prev) => {
      if (!prev) {
        setActiveMobilePanel(null);
      }
      setInspectorOpen(prev ? true : false);
      return !prev;
    });
  }, []);

  const openAvatarSwitcherPanel = useCallback(() => {
    if (isConversationCompactLayout) {
      setActiveMobilePanel('avatar-switcher');
      return;
    }
    setMobileAvatarPickerOpen(true);
  }, [isConversationCompactLayout]);

  const openGuidePanel = useCallback(() => {
    if (isConversationCompactLayout) {
      setActiveMobilePanel('guide');
      return;
    }
    setTab('guide');
  }, [isConversationCompactLayout]);

  const closeActiveMobilePanel = useCallback(() => {
    if (isConversationCompactLayout) {
      setActiveMobilePanel(null);
      return;
    }
    setMobileAvatarPickerOpen(false);
    setTab('workspace');
  }, [isConversationCompactLayout]);

  const guideDialogOpen = isConversationCompactLayout ? activeMobilePanel === 'guide' : tab === 'guide';
  const avatarPickerDialogOpen = isConversationCompactLayout
    ? activeMobilePanel === 'avatar-switcher'
    : mobileAvatarPickerOpen;

  const persistWorkspaceFocusAvatar = useCallback((avatarId: string | null) => {
    if (typeof window === 'undefined') return;
    if (avatarId) {
      window.localStorage.setItem(pendingManagerFocusStorageKey, avatarId);
      return;
    }
    window.localStorage.removeItem(pendingManagerFocusStorageKey);
  }, [pendingManagerFocusStorageKey]);

  const openOverviewWorkspace = useCallback(() => {
    persistWorkspaceFocusAvatar(selectedAvatarDisplay?.id ?? null);
    navigate(`/teams/${teamId}/digital-avatars/overview`);
  }, [navigate, persistWorkspaceFocusAvatar, selectedAvatarDisplay?.id, teamId]);

  const openAuditWorkspace = useCallback(() => {
    persistWorkspaceFocusAvatar(selectedAvatarDisplay?.id ?? null);
    navigate(`/teams/${teamId}/digital-avatars/audit`);
  }, [navigate, persistWorkspaceFocusAvatar, selectedAvatarDisplay?.id, teamId]);

  const compactConversationActions = isConversationCompactLayout ? (
    <div className="grid grid-cols-2 gap-2">
      <Button
        variant="outline"
        size="sm"
        className="h-10 rounded-[16px] justify-start px-3 text-[12px]"
        onClick={openAvatarSwitcherPanel}
      >
        <Users className="mr-2 h-3.5 w-3.5" />
        {selectedAvatarDisplay
          ? t('digitalAvatar.workspace.switchAvatar', '切换分身')
          : t('digitalAvatar.workspace.selectAvatar', '选择分身')}
      </Button>
      <Button
        variant="outline"
        size="sm"
        className={`h-10 rounded-[16px] justify-start px-3 text-[12px] transition-all ${
          inspectorOpen
            ? 'border-primary/46 bg-primary/12 text-primary shadow-[0_12px_24px_hsl(var(--primary))/0.14]'
            : ''
        }`}
        onClick={() => openInspectorPanel('overview')}
      >
        <Check className="mr-2 h-3.5 w-3.5" />
        {t('digitalAvatar.actions.showConsole', '打开控制台')}
      </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-10 rounded-[16px] justify-start px-3 text-[12px]"
          onClick={openOverviewWorkspace}
        >
        <Clock3 className="mr-2 h-3.5 w-3.5" />
        {t('digitalAvatar.workspace.mobilePendingEntry', '看待处理')}
      </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-10 rounded-[16px] justify-start px-3 text-[12px]"
          onClick={openAuditWorkspace}
        >
        <ShieldAlert className="mr-2 h-3.5 w-3.5" />
        {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
      </Button>
    </div>
  ) : null;

  const compactConversationStage = isConversationCompactLayout ? (
    <Card className={`${WORKSPACE_PANEL_CLASS} flex h-full min-h-0 flex-col overflow-hidden`}>
      <CardHeader className="border-b border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.18] px-4 py-3 dark:bg-[hsl(var(--ui-surface-panel-muted))/0.2]">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <CardTitle className="text-[13px] tracking-[-0.01em] text-foreground">
              {t('digitalAvatar.workspace.focusTitle', '管理 Agent 对话')}
            </CardTitle>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-[11px] leading-5 text-muted-foreground">
              <span>{getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset'))}</span>
              {managerAgent ? (
                <AgentTypeBadge type={resolveAgentVisualType(managerAgent)} className="shrink-0" />
              ) : null}
              {selectedAvatarDisplay ? (
                <>
                  <span className="text-border/70">·</span>
                  <span className="truncate">{selectedAvatarDisplay.name}</span>
                  <AvatarTypeBadge type={selectedAvatarType} className="shrink-0" />
                </>
              ) : null}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button
              variant="ghost"
              size="sm"
              className="h-8 rounded-full px-3 text-[11px] text-muted-foreground"
              onClick={openGuidePanel}
            >
              {t('digitalAvatar.tabs.guide', '使用指南')}
            </Button>
            <Button
              size="sm"
              className="h-8 rounded-full px-3 text-[11px]"
              onClick={toggleFocusMode}
            >
              {t('digitalAvatar.actions.focusConversation', '专注对话')}
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent className="min-h-0 flex-1 overflow-hidden p-0">
        {!effectiveManagerAgentId ? (
          <div className="flex h-full items-center justify-center">
            <div className="space-y-2 text-center text-caption text-muted-foreground">
              <p>{t('digitalAvatar.states.noManagerAgent')}</p>
              {canManage ? (
                <Button size="sm" variant="outline" onClick={openEcosystem}>
                  <ExternalLink className="mr-1 h-3.5 w-3.5" />
                  {t('digitalAvatar.actions.openEcosystem')}
                </Button>
              ) : null}
            </div>
          </div>
        ) : (
          <ChatConversation
            sessionId={managerSessionId}
            agentId={effectiveManagerAgentId}
            agentName={managerConversationAgentName}
            agent={managerAgent || undefined}
            headerVariant="compact"
            inputQuickActionGroups={managerQuickActionGroups}
            teamId={teamId}
            createSession={createManagerSession}
            onSessionCreated={onManagerSessionCreated}
            onRuntimeEvent={handleRuntimeEvent}
            onProcessingChange={setManagerProcessing}
            composeRequest={managerComposeRequest}
          />
        )}
      </CardContent>
    </Card>
  ) : null;

  const compactConversationRail = isConversationCompactLayout ? (
    <ManagementRail
      title={t('digitalAvatar.workspace.currentContextTitle', '当前工作上下文')}
      description={t(
        'digitalAvatar.workspace.currentContextHint',
        '分身配置、发布信息和治理入口退到辅助层，不再和对话主舞台抢首屏。',
      )}
      action={
        <Button
          variant="ghost"
          size="sm"
          className="h-8 rounded-full px-3 text-[11px] text-muted-foreground"
          onClick={openAuditWorkspace}
        >
          {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
        </Button>
      }
    >
      <div className="overflow-hidden rounded-[20px] border border-border/60 bg-[hsl(var(--ui-surface-panel))/0.7]">
        {[
          {
            label: t('digitalAvatar.labels.managerAgent'),
            value: getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset')),
          },
          {
            label: t('digitalAvatar.workspace.summaryPublish', '发布地址'),
            value: selectedAvatarPublicUrl || publishPath || t('digitalAvatar.labels.unset'),
            breakAll: true,
          },
          {
            label: t('digitalAvatar.labels.documentAccess'),
            value: formatDocumentAccessMode(selectedAvatarDocumentAccessMode, t),
          },
          {
            label: t('digitalAvatar.workspace.summaryType', '分身类型'),
            value: selectedAvatarDisplay ? selectedAvatarOutputFormLabel : t('digitalAvatar.workspace.groupModeBadge', '全局'),
          },
        ].map((item, index, items) => (
          <div
            key={item.label}
            className={`flex items-start justify-between gap-3 px-3 py-2.5 ${
              index !== items.length - 1 ? 'border-b border-border/60' : ''
            }`}
          >
            <div className="shrink-0 text-[10px] uppercase tracking-[0.12em] text-muted-foreground/72">
              {item.label}
            </div>
            <div
              className={`min-w-0 text-right text-[12px] font-semibold leading-5 text-foreground ${
                item.breakAll ? 'break-all' : ''
              }`}
            >
              {item.value}
            </div>
          </div>
        ))}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground">
        <button
          type="button"
          className="inline-flex h-8 items-center rounded-full border border-border/60 bg-background/65 px-3 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground"
          onClick={() => navigate(`/teams/${teamId}/digital-avatars/policies`)}
        >
          {t('digitalAvatar.actions.policyCenter', '风险策略')}
        </button>
        <button
          type="button"
          className="inline-flex h-8 items-center rounded-full border border-border/60 bg-background/65 px-3 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground disabled:pointer-events-none disabled:opacity-70"
          onClick={() => loadAvatars(false)}
          disabled={refreshing}
        >
          {refreshing ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="mr-1 h-3.5 w-3.5" />}
          {t('digitalAvatar.actions.refresh')}
        </button>
        {canManage ? (
          <button
            type="button"
            className="inline-flex h-8 items-center rounded-full border border-border/60 bg-background/65 px-3 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground"
            onClick={() => setCreateManagerOpen(true)}
          >
            <Users className="mr-1 h-3.5 w-3.5" />
            {t('digitalAvatar.actions.createManagerGroup', '新建管理组')}
          </button>
        ) : null}
        {canManage ? (
          <button
            type="button"
            className="inline-flex h-8 items-center rounded-full border border-border/60 bg-background/65 px-3 text-[11px] font-medium text-muted-foreground transition-colors hover:text-foreground"
            onClick={() => {
              if (!selectedManagerGroupId) {
                addToast('error', t('digitalAvatar.states.noManagerAgent'));
                return;
              }
              setCreateOpen(true);
            }}
          >
            <Plus className="mr-1 h-3.5 w-3.5" />
            {t('digitalAvatar.actions.createAvatarAdvanced', '高级创建分身')}
          </button>
        ) : null}
      </div>
    </ManagementRail>
  ) : null;

  const compactConsoleSnapshot = isConversationCompactLayout ? (
    <div className="mb-3 rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-muted))/0.9] p-3 dark:bg-[hsl(var(--ui-surface-panel-muted))/0.86]">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-[10px] font-semibold tracking-[0.14em] text-muted-foreground uppercase">
            {t('digitalAvatar.workspace.currentContextTitle', '当前工作上下文')}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2">
            <span className="truncate text-[13px] font-semibold text-foreground">
              {selectedAvatarDisplay?.name || t('digitalAvatar.workspace.groupModeShort', '管理组全局模式')}
            </span>
            <span className="inline-flex min-h-6 items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
              {mobileWorkspaceContextStatus}
            </span>
          </div>
        </div>
        {selectedAvatarDisplay ? <AvatarTypeBadge type={selectedAvatarType} className="shrink-0" /> : null}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <span className="inline-flex min-h-7 items-center rounded-full border border-[hsl(var(--status-warning-text))/0.16] bg-status-warning-bg px-2.5 py-1 text-[10px] font-semibold text-status-warning-text">
          {t('digitalAvatar.labels.pendingCount', '待处理')} {mobileWorkspacePendingCount}
        </span>
        <span className="inline-flex min-h-7 items-center rounded-full border border-border/60 bg-background/70 px-2.5 py-1 text-[10px] font-semibold text-foreground">
          {formatDocumentAccessMode(selectedAvatarDocumentAccessMode, t)}
        </span>
        <span className="inline-flex min-h-7 items-center rounded-full border border-border/60 bg-background/70 px-2.5 py-1 text-[10px] font-semibold text-foreground">
          {t('digitalAvatar.list.lastActivity', '最近活动')} ·{' '}
          {selectedAvatarDisplay ? selectedAvatarLastActivityLabel : t('digitalAvatar.labels.managerGroup', '管理 Agent 组')}
        </span>
      </div>
      <div className="mt-2 rounded-[14px] border border-border/60 bg-background/65 px-3 py-2">
        <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/72">
          {t('digitalAvatar.workspace.summaryPublish', '发布地址')}
        </div>
        <div className="mt-1 break-all text-[12px] font-semibold leading-5 text-foreground">
          {selectedAvatarPublicUrl || publishPath || t('digitalAvatar.labels.unset')}
        </div>
      </div>
    </div>
  ) : null;


  return (
    <div className="flex h-[calc(100vh-18px)] min-h-[720px] flex-col gap-2 overflow-hidden bg-[linear-gradient(180deg,hsl(var(--background))_0%,hsl(var(--muted))/0.1_100%)] px-3 pb-3 pt-2">
      <div className={focusMode ? '' : `${WORKSPACE_SHELL_CLASS}`}>
        {focusMode ? (
          <div className={`${WORKSPACE_PANEL_CLASS} flex items-center justify-between gap-3 px-4 py-3`}>
            <div className="flex min-w-0 items-center gap-2 overflow-hidden">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[10px] border border-primary/14 bg-primary/10 text-primary">
                <UserRound className="h-3.5 w-3.5" />
              </div>
              <div className="min-w-0 flex items-center gap-2 overflow-hidden whitespace-nowrap text-[12px] leading-5">
                <span className="shrink-0 font-semibold text-foreground">{t('digitalAvatar.title')}</span>
                <span className="shrink-0 text-border">·</span>
                <span className="truncate text-muted-foreground">
                  {getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset'))}
                </span>
                {managerAgent ? (
                  <AgentTypeBadge type={resolveAgentVisualType(managerAgent)} className="shrink-0" />
                ) : null}
                {selectedAvatarDisplay ? (
                  <>
                    <AvatarTypeBadge type={selectedAvatarType} className="shrink-0" />
                    <span className="truncate text-muted-foreground">{selectedAvatarDisplay.name}</span>
                    <span className="shrink-0 rounded-[8px] border border-border/80 bg-secondary/55 px-2 py-0.5 text-[10px] text-muted-foreground">
                      {selectedAvatarStatusLabel}
                    </span>
                  </>
                ) : (
                  <span className="shrink-0 rounded-[8px] border border-border/80 bg-secondary/55 px-2 py-0.5 text-[10px] text-muted-foreground">
                    {t('digitalAvatar.states.noAvatarSelected')}
                  </span>
                )}
              </div>
            </div>
            <Button size="sm" className="h-8 px-3" onClick={toggleFocusMode}>
              {t('digitalAvatar.actions.exitFocus', '退出专注')}
            </Button>
          </div>
        ) : isCompactInspectorLayout ? (
          isConversationCompactLayout ? null : (
          <div className={`${WORKSPACE_PANEL_CLASS} space-y-3 px-4 py-4`}>
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex items-start gap-3">
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[16px] border border-primary/14 bg-primary/10 text-primary shadow-[0_8px_18px_hsl(var(--primary))/0.08]">
                  <UserRound className="h-4 w-4" />
                </div>
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-1.5 text-[9px] font-medium uppercase tracking-[0.12em] text-muted-foreground/75">
                    <span>{t('digitalAvatar.title')}</span>
                    <span className="text-border">·</span>
                    <span>{getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset'))}</span>
                  </div>
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <h2 className="truncate text-[15px] font-semibold tracking-[-0.02em] text-foreground">
                      {mobileWorkspaceContextTitle}
                    </h2>
                    <span className="inline-flex min-h-6 items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
                      {mobileWorkspaceContextStatus}
                    </span>
                    {selectedAvatarDisplay ? <AvatarTypeBadge type={selectedAvatarType} className="shrink-0" /> : null}
                  </div>
                  <p className="line-clamp-2 text-[12px] leading-5 text-muted-foreground">
                    {mobileWorkspaceContextDescription}
                  </p>
                </div>
              </div>
              <div className="flex shrink-0 flex-col items-stretch gap-2">
                <Button variant="outline" size="sm" className="h-8 rounded-full px-3 text-[12px]" onClick={() => setMobileAvatarPickerOpen(true)}>
                  {selectedAvatarDisplay
                    ? t('digitalAvatar.workspace.switchAvatar', '切换分身')
                    : t('digitalAvatar.workspace.selectAvatar', '选择分身')}
                </Button>
                <Button size="sm" className="h-8 rounded-full px-3 text-[12px]" onClick={toggleFocusMode}>
                  {t('digitalAvatar.actions.focusConversation', '专注对话')}
                </Button>
              </div>
            </div>
            <div className="grid grid-cols-3 gap-2">
              <div className="rounded-[18px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.9] px-3 py-2.5">
                <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/75">
                  {t('digitalAvatar.labels.currentAvatar', '当前分身')}
                </div>
                <div className="mt-1 truncate text-[12px] font-semibold text-foreground">
                  {selectedAvatarDisplay?.slug ? `/p/${selectedAvatarDisplay.slug}` : t('digitalAvatar.workspace.groupModeBadge', '全局')}
                </div>
              </div>
              <div className="rounded-[18px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.9] px-3 py-2.5">
                <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/75">
                  {t('digitalAvatar.labels.pendingCount', '待处理')}
                </div>
                <div className="mt-1 text-[12px] font-semibold text-foreground">{mobileWorkspacePendingCount}</div>
              </div>
              <div className="rounded-[18px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.9] px-3 py-2.5">
                <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/75">
                  {t('digitalAvatar.list.lastActivity', '最近活动')}
                </div>
                <div className="mt-1 truncate text-[12px] font-semibold text-foreground">
                  {selectedAvatarDisplay ? selectedAvatarLastActivityLabel : t('digitalAvatar.labels.managerGroup', '管理 Agent 组')}
                </div>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <Button variant="outline" size="sm" className="h-10 rounded-[16px] justify-start px-3 text-[12px]" onClick={openOverviewWorkspace}>
                <Clock3 className="mr-2 h-3.5 w-3.5" />
                {t('digitalAvatar.workspace.mobilePendingEntry', '看待处理')}
              </Button>
              <Button variant="outline" size="sm" className="h-10 rounded-[16px] justify-start px-3 text-[12px]" onClick={openAuditWorkspace}>
                <ShieldAlert className="mr-2 h-3.5 w-3.5" />
                {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
              </Button>
              <Button
                variant="outline"
                size="sm"
                className={`h-10 rounded-[16px] justify-start px-3 text-[12px] transition-all ${
                  inspectorOpen
                    ? 'border-primary/46 bg-primary/12 text-primary shadow-[0_12px_24px_hsl(var(--primary))/0.14]'
                    : ''
                }`}
                onClick={() => openInspectorPanel('overview')}
              >
                <Check className="mr-2 h-3.5 w-3.5" />
                {t('digitalAvatar.actions.showConsole', '打开控制台')}
              </Button>
              <Button variant="outline" size="sm" className="h-10 rounded-[16px] justify-start px-3 text-[12px]" onClick={() => setTab('guide')}>
                <ExternalLink className="mr-2 h-3.5 w-3.5" />
                {t('digitalAvatar.tabs.guide', '使用指南')}
              </Button>
            </div>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-[11px] text-muted-foreground">
              <button type="button" className={INSPECTOR_ACTION_LINK_CLASS} onClick={() => navigate(`/teams/${teamId}/digital-avatars/policies`)}>
                {t('digitalAvatar.actions.policyCenter', '风险策略')}
              </button>
              <button type="button" className={INSPECTOR_ACTION_LINK_CLASS} onClick={() => loadAvatars(false)} disabled={refreshing}>
                {refreshing ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="mr-1 h-3.5 w-3.5" />}
                {t('digitalAvatar.actions.refresh')}
              </button>
              {canManage ? (
                <button type="button" className={INSPECTOR_ACTION_LINK_CLASS} onClick={() => setCreateManagerOpen(true)}>
                  <Users className="mr-1 h-3.5 w-3.5" />
                  {t('digitalAvatar.actions.createManagerGroup', '新建管理组')}
                </button>
              ) : null}
              {canManage ? (
                <button
                  type="button"
                  className={INSPECTOR_ACTION_LINK_CLASS}
                  onClick={() => {
                    if (!selectedManagerGroupId) {
                      addToast('error', t('digitalAvatar.states.noManagerAgent'));
                      return;
                    }
                    setCreateOpen(true);
                  }}
                >
                  <Plus className="mr-1 h-3.5 w-3.5" />
                  {t('digitalAvatar.actions.createAvatarAdvanced', '高级创建分身')}
                </button>
              ) : null}
            </div>
          </div>
          )
        ) : (
          <div className={CONTROL_DECK_CLASS}>
            <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
              <div className="min-w-0 flex items-start gap-2.5">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[16px] border border-primary/14 bg-primary/10 text-primary shadow-[0_8px_18px_hsl(var(--primary))/0.08]">
                  <UserRound className="h-3.5 w-3.5" />
                </div>
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-1.5 text-[9px] font-medium uppercase tracking-[0.12em] text-muted-foreground/75">
                    <span>{t('digitalAvatar.title')}</span>
                    <span className="text-border">·</span>
                    <span>{getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset'))}</span>
                  </div>
                  <div className="mt-1 flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
                    <h2 className="text-[15px] font-semibold tracking-[-0.02em] text-foreground">
                      {selectedAvatarDisplay
                        ? selectedAvatarDisplay.name
                        : t('digitalAvatar.workspace.managerControlRoomTitle', '{{name}} · 管理控制室', {
                            name: getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset')),
                          })}
                    </h2>
                    {selectedAvatarDisplay?.slug ? (
                      <span className="truncate text-[12px] font-medium tracking-[0.02em] text-muted-foreground">
                        /p/{selectedAvatarDisplay.slug}
                      </span>
                    ) : null}
                  </div>
                </div>
              </div>
              <div className="flex flex-wrap items-center gap-x-4 gap-y-2 lg:justify-end">
                <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
                  <Button variant="ghost" size="sm" className={CONTROL_ROOM_TOOLBAR_BUTTON_CLASS} onClick={() => setTab('guide')}>
                    {t('digitalAvatar.tabs.guide', '使用指南')}
                  </Button>
                  <Button variant="ghost" size="sm" className={CONTROL_ROOM_TOOLBAR_BUTTON_CLASS} onClick={openOverviewWorkspace}>
                    {t('digitalAvatar.actions.overview', '治理总览')}
                  </Button>
                  <Button variant="ghost" size="sm" className={CONTROL_ROOM_TOOLBAR_BUTTON_CLASS} onClick={openAuditWorkspace}>
                    {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
                  </Button>
                  <Button variant="ghost" size="sm" className={CONTROL_ROOM_TOOLBAR_BUTTON_CLASS} onClick={() => navigate(`/teams/${teamId}/digital-avatars/policies`)}>
                    {t('digitalAvatar.actions.policyCenter', '风险策略')}
                  </Button>
                  <Button variant="ghost" size="sm" className={CONTROL_ROOM_TOOLBAR_BUTTON_CLASS} onClick={() => loadAvatars(false)} disabled={refreshing}>
                    {refreshing ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="mr-1 h-3.5 w-3.5" />}
                    {t('digitalAvatar.actions.refresh')}
                  </Button>
                </div>
                {canManage ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <Button variant="ghost" size="sm" className={CONTROL_ROOM_TOOLBAR_BUTTON_CLASS} onClick={() => setCreateManagerOpen(true)}>
                      <Users className="mr-1 h-3.5 w-3.5" />
                      {t('digitalAvatar.actions.createManagerGroup', '新建管理组')}
                    </Button>
                    <Button
                      size="sm"
                      className={`h-8.5 rounded-full px-4 shadow-[0_10px_20px_hsl(var(--primary))/0.12] ${AVATAR_PRIMARY_BUTTON_CLASS}`}
                      onClick={() => {
                        if (!selectedManagerGroupId) {
                          addToast('error', t('digitalAvatar.states.noManagerAgent'));
                          return;
                        }
                        setCreateOpen(true);
                      }}
                    >
                      <Plus className="mr-1.5 h-3.5 w-3.5" />
                      {t('digitalAvatar.actions.createAvatarAdvanced', '高级创建分身')}
                    </Button>
                  </div>
                ) : null}
              </div>
            </div>
          </div>
        )}
      </div>

      <Dialog
        open={guideDialogOpen}
        onOpenChange={(open) => {
          if (isConversationCompactLayout) {
            setActiveMobilePanel(open ? 'guide' : null);
            return;
          }
          setTab(open ? 'guide' : 'workspace');
        }}
      >
        <DialogContent className="max-h-[88vh] overflow-hidden p-0 sm:max-w-5xl">
          <DialogHeader className="border-b border-border/60 px-5 py-4">
            <DialogTitle>{t('digitalAvatar.tabs.guide', '使用指南')}</DialogTitle>
          </DialogHeader>
          <div className="max-h-[calc(88vh-76px)] overflow-y-auto">
            <DigitalAvatarGuide
              teamId={teamId}
              currentAvatarId={selectedAvatarDisplay?.id ?? null}
              canSendCommand={Boolean(effectiveManagerAgentId)}
              onCopyCommand={copyGuideCommand}
              onSendCommand={sendGuideCommandToManager}
            />
          </div>
        </DialogContent>
      </Dialog>
      <Dialog
        open={avatarPickerDialogOpen}
        onOpenChange={(open) => {
          if (isConversationCompactLayout) {
            setActiveMobilePanel(open ? 'avatar-switcher' : null);
            return;
          }
          setMobileAvatarPickerOpen(open);
        }}
      >
        <DialogContent className="flex h-[100dvh] max-h-[100dvh] w-screen max-w-none flex-col overflow-hidden rounded-none p-0 sm:h-auto sm:max-h-[88vh] sm:w-full sm:max-w-xl sm:rounded-[28px]">
          <DialogHeader className="border-b border-border/60 px-5 py-4">
            <DialogTitle>{t('digitalAvatar.workspace.selectAvatar', '选择分身')}</DialogTitle>
          </DialogHeader>
          <div className="flex min-h-0 flex-1 flex-col overflow-hidden px-4 pb-4 pt-3">
            <div className="space-y-3">
              <div className="space-y-1.5">
                <p className="text-[11px] text-muted-foreground">
                  {t('digitalAvatar.labels.managerGroup', '管理 Agent 组')}
                </p>
                {managerGroupOptions.length === 0 ? (
                  <div className="rounded-[16px] border border-dashed border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-muted))/0.45] px-3 py-3 text-[12px] text-muted-foreground">
                    {canManage
                      ? t(
                          'digitalAvatar.states.noManagerAgentHint',
                          '还没有管理组。请先使用顶部“新建管理组”创建一个专用管理 Agent。',
                        )
                      : t('digitalAvatar.states.noManagerAgent')}
                  </div>
                ) : (
                  <UiSelect
                    value={selectedManagerGroupId || ''}
                    onValueChange={(value) => {
                      setSelectedAvatarId(null);
                      setBootstrapManagerAgentId(value);
                    }}
                  >
                    <SelectTrigger className="h-11 rounded-[18px] border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-strong))/0.94] px-3 text-[13px] font-medium text-foreground shadow-none ring-0 focus:ring-0 focus:ring-offset-0 dark:bg-[hsl(var(--ui-surface-panel-strong))/0.88]">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent className="rounded-[18px] border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-strong))/0.98] shadow-[0_20px_44px_hsl(var(--ui-shadow))/0.16] dark:bg-[hsl(var(--ui-surface-panel-strong))/0.96]">
                      {managerGroupOptions.map((agent) => (
                        <SelectItem
                          key={agent.id}
                          value={agent.id}
                          className="rounded-[14px] py-2.5 pl-9 pr-3 text-[13px] font-medium text-foreground focus:bg-[hsl(var(--ui-surface-selected))/0.8] focus:text-foreground"
                        >
                          {getAgentDisplayName(agent, agent.name)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </UiSelect>
                )}
              </div>
              <div className="flex items-center gap-5 border-b border-border/60 pb-2">
                {([
                  { key: 'all', label: t('digitalAvatar.filters.allShort', '全部') },
                  { key: 'external', label: t('digitalAvatar.filters.externalShort', '外部') },
                  { key: 'internal', label: t('digitalAvatar.filters.internalShort', '内部') },
                ] as { key: AvatarFilter; label: string }[]).map((item) => (
                  <button
                    key={item.key}
                    className={`${BARE_BUTTON_CLASS} relative flex h-8 min-w-0 items-center justify-center px-0 text-[12px] font-semibold leading-none whitespace-nowrap transition-colors ${
                      filter === item.key ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'
                    }`}
                    onClick={() => setFilter(item.key)}
                  >
                    {filter === item.key ? (
                      <span className="absolute inset-x-0 -bottom-[9px] mx-auto h-0.5 w-10 rounded-full bg-foreground/90" />
                    ) : null}
                    {item.label}
                  </button>
                ))}
              </div>
            </div>
            <div className="scrollbar-ghost mt-4 min-h-0 flex-1 overflow-y-auto">
              <div className="space-y-4">
                <button
                  type="button"
                  className={`${BARE_BUTTON_CLASS} w-full rounded-[20px] px-3.5 py-3 text-left transition-all ${
                    globalModeActive
                      ? 'bg-[hsl(var(--ui-surface-selected))/0.64] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-strong))/0.16,0_8px_18px_hsl(var(--ui-shadow))/0.04]'
                      : 'bg-[hsl(var(--ui-surface-panel-muted))/0.32] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.12] hover:bg-[hsl(var(--ui-surface-panel-muted))/0.46] hover:shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.18]'
                  }`}
                  onClick={() => {
                    setSelectedAvatarId(null);
                    closeActiveMobilePanel();
                  }}
                >
                  <div className="space-y-2">
                    <div className="flex items-start justify-between gap-2">
                      <p className="pr-2 text-[12px] font-semibold tracking-[0.01em] text-foreground">
                        {t('digitalAvatar.workspace.groupModeShort', '管理组全局模式')}
                      </p>
                      <span className="inline-flex min-h-6 items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel-strong))/0.8] px-2.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
                        {t('digitalAvatar.workspace.groupModeBadge', '全局')}
                      </span>
                    </div>
                    <p className="text-[11px] leading-5 text-muted-foreground">
                      {t('digitalAvatar.workspace.groupModeHintShort', '盘点当前管理组、规划新分身与处理全局治理。')}
                    </p>
                  </div>
                </button>
                {avatarSections.map((section) => (
                  <div key={section.key} className="space-y-2.5">
                    <div className="flex items-center justify-between gap-2 px-1">
                      <p className="text-[13px] font-semibold leading-none tracking-[-0.01em] text-foreground">{section.title}</p>
                      <span className="inline-flex min-w-5 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.92] px-1.5 py-0.5 text-[10px] text-muted-foreground dark:bg-[hsl(var(--ui-surface-panel-muted))/0.72]">
                        {section.items.length}
                      </span>
                    </div>
                    <div className="space-y-1.5 pt-0.5">
                      {section.items.map((avatar) => {
                        const selected = avatar.id === selectedAvatarId;
                        const status = normalizeAvatarStatus(avatar);
                        const projection = avatarProjectionMap[avatar.id];
                        const pendingCount = getAvatarProjectionPendingCount(projection);
                        const activityAt = projection?.portalUpdatedAt || avatar.updatedAt || avatar.createdAt;
                        const extensionCount =
                          avatar.effectivePublicConfig?.effectiveAllowedExtensions?.length
                          ?? avatar.allowedExtensions?.length
                          ?? 0;
                        const skillCount =
                          avatar.effectivePublicConfig?.effectiveAllowedSkillIds?.length
                          ?? avatar.allowedSkillIds?.length
                          ?? 0;
                        const statusLabel =
                          status === 'published'
                            ? t('digitalAvatar.status.published', '已发布')
                            : status === 'draft'
                              ? t('digitalAvatar.status.draft', '草稿')
                              : status === 'disabled'
                                ? t('digitalAvatar.status.disabled', '已停用')
                                : status === 'archived'
                                  ? t('digitalAvatar.status.archived', '已归档')
                                  : t('digitalAvatar.labels.unset');
                        return (
                          <button
                            key={avatar.id}
                            className={`${BARE_BUTTON_CLASS} relative w-full rounded-[20px] px-3.5 py-3 text-left transition-all ${
                              selected
                                ? 'bg-[hsl(var(--ui-surface-selected))/0.66] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-strong))/0.16,0_8px_18px_hsl(var(--ui-shadow))/0.04]'
                                : 'bg-[hsl(var(--ui-surface-panel-muted))/0.28] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.1] hover:bg-[hsl(var(--ui-surface-panel-muted))/0.42] hover:shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.16]'
                            }`}
                            onClick={() => {
                              setSelectedAvatarId(avatar.id);
                              closeActiveMobilePanel();
                            }}
                          >
                            <div className="min-w-0 space-y-2">
                              <div className="flex items-start justify-between gap-2">
                                <p className="truncate pr-1 text-[12px] font-semibold tracking-[0.01em] text-foreground">{avatar.name}</p>
                                <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                                  {pendingCount > 0 ? (
                                    <span className="inline-flex min-w-[60px] justify-center h-6 items-center rounded-full border border-[hsl(var(--status-warning-text))/0.2] bg-status-warning-bg px-2.5 text-[10px] font-semibold leading-none tracking-[0.01em] text-status-warning-text">
                                      {t('digitalAvatar.labels.pendingCount', '待处理')} {pendingCount}
                                    </span>
                                  ) : null}
                                  <span className={`inline-flex min-w-[52px] justify-center h-6 items-center rounded-full px-2.5 text-[10px] font-semibold leading-none tracking-[0.01em] ${avatarStatusBadgeClass(status)}`}>
                                    {statusLabel}
                                  </span>
                                </div>
                              </div>
                              <div className="text-[10px] leading-5 text-muted-foreground">
                                {t('digitalAvatar.list.capabilityCounts', '扩展 {{extensions}} · 技能 {{skills}}', {
                                  extensions: extensionCount,
                                  skills: skillCount,
                                })}
                              </div>
                              <div className="flex items-center gap-2 text-[10px] text-muted-foreground/80">
                                <span>
                                  {t('digitalAvatar.list.lastActivity', '最近活动')} · {activityAt ? formatRelativeTime(activityAt) : t('digitalAvatar.labels.unset')}
                                </span>
                              </div>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
        </DialogContent>
      </Dialog>
      <>
        <div className={`${focusMode ? '' : CONTROL_ROOM_CHROME_CLASS} min-h-0 flex-1 overflow-hidden`}>
        {!focusMode && isConversationCompactLayout ? (
          <MobileWorkspaceShell
            summary={
              <ContextSummaryBar
                eyebrow={t('digitalAvatar.title')}
                title={mobileWorkspaceContextTitle}
                description={mobileWorkspaceContextDescription}
                badge={
                  <div className="flex items-center gap-2">
                    <span className="inline-flex min-h-6 items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
                      {mobileWorkspaceContextStatus}
                    </span>
                    {selectedAvatarDisplay ? (
                      <AvatarTypeBadge type={selectedAvatarType} className="shrink-0" />
                    ) : null}
                  </div>
                }
                metrics={[
                  {
                    label: t('digitalAvatar.labels.currentAvatar', '当前分身'),
                    value: selectedAvatarDisplay?.slug
                      ? `/p/${selectedAvatarDisplay.slug}`
                      : t('digitalAvatar.workspace.groupModeBadge', '全局'),
                  },
                  {
                    label: t('digitalAvatar.labels.pendingCount', '待处理'),
                    value: mobileWorkspacePendingCount,
                  },
                  {
                    label: t('digitalAvatar.list.lastActivity', '最近活动'),
                    value: selectedAvatarDisplay
                      ? selectedAvatarLastActivityLabel
                      : t('digitalAvatar.labels.managerGroup', '管理 Agent 组'),
                  },
                  {
                    label: t('digitalAvatar.labels.managerAgent'),
                    value: getAgentDisplayName(managerAgent, t('digitalAvatar.labels.unset')),
                  },
                ]}
              />
            }
            actions={compactConversationActions}
            stage={compactConversationStage}
            rail={compactConversationRail}
          />
        ) : (
        <div className={`min-h-0 h-full grid gap-2.5 ${focusMode ? 'grid-cols-1' : inspectorOpen ? 'grid-cols-1 lg:grid-cols-[272px_minmax(0,1fr)_344px]' : 'grid-cols-1 lg:grid-cols-[272px_minmax(0,1fr)]'}`}>
          {!focusMode && !isCompactInspectorLayout && (
          <Card className={`${AVATAR_NAV_PANEL_CLASS} min-h-0 flex flex-col overflow-hidden`}>
            <CardHeader className="bg-[hsl(var(--ui-surface-panel-muted))/0.34] px-4 pb-3 pt-4 dark:bg-[hsl(var(--ui-surface-panel-muted))/0.28]">
              <CardTitle className="text-sm flex items-center justify-between">
                <span className="text-foreground">{t('digitalAvatar.list.title')}</span>
                <span className="text-caption font-normal text-muted-foreground">{visibleAvatars.length}</span>
              </CardTitle>
              <div className="space-y-1.5">
                <p className="text-[11px] text-muted-foreground">
                  {t('digitalAvatar.labels.managerGroup', '管理 Agent 组')}
                </p>
                {managerGroupOptions.length === 0 ? (
                  <div className="space-y-2">
                    <div className="rounded-[16px] border border-dashed border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-muted))/0.45] px-2.5 py-2.5 text-[11px] text-muted-foreground">
                      {canManage
                        ? t(
                            'digitalAvatar.states.noManagerAgentHint',
                            '还没有管理组。请先使用顶部“新建管理组”创建一个专用管理 Agent。'
                          )
                        : t('digitalAvatar.states.noManagerAgent')}
                    </div>
                  </div>
                ) : (
                  <>
                    <UiSelect
                      value={selectedManagerGroupId || ''}
                      onValueChange={(value) => {
                        setSelectedAvatarId(null);
                        setBootstrapManagerAgentId(value);
                      }}
                    >
                      <SelectTrigger className="h-11 rounded-[18px] border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-strong))/0.94] px-3 text-[13px] font-medium text-foreground shadow-none ring-0 focus:ring-0 focus:ring-offset-0 dark:bg-[hsl(var(--ui-surface-panel-strong))/0.88]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent className="rounded-[18px] border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-strong))/0.98] shadow-[0_20px_44px_hsl(var(--ui-shadow))/0.16] dark:bg-[hsl(var(--ui-surface-panel-strong))/0.96]">
                        {managerGroupOptions.map((agent) => (
                          <SelectItem
                            key={agent.id}
                            value={agent.id}
                            className="rounded-[14px] py-2.5 pl-9 pr-3 text-[13px] font-medium text-foreground focus:bg-[hsl(var(--ui-surface-selected))/0.8] focus:text-foreground"
                          >
                            {getAgentDisplayName(agent, agent.name)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </UiSelect>
                  </>
                )}
              </div>
                  <div className="mt-3 flex items-center gap-5 pb-1">
                    {([
                      { key: 'all', label: t('digitalAvatar.filters.allShort', '全部') },
                      { key: 'external', label: t('digitalAvatar.filters.externalShort', '外部') },
                      { key: 'internal', label: t('digitalAvatar.filters.internalShort', '内部') },
                ] as { key: AvatarFilter; label: string }[]).map((item) => (
                  <button
                    key={item.key}
                    className={`${BARE_BUTTON_CLASS} relative flex h-8 min-w-0 items-center justify-center px-0 text-[12px] font-semibold leading-none whitespace-nowrap transition-colors ${
                      filter === item.key
                        ? 'text-foreground'
                        : 'text-muted-foreground hover:text-foreground'
                    }`}
                    onClick={() => setFilter(item.key)}
                  >
                    {filter === item.key ? (
                      <span className="absolute inset-x-0 -bottom-[9px] mx-auto h-0.5 w-10 rounded-full bg-foreground/90" />
                    ) : null}
                    {item.label}
                  </button>
                ))}
              </div>
            </CardHeader>
            <CardContent className="min-h-0 flex-1 overflow-y-auto px-0 pb-2 pt-3 scrollbar-ghost">
              {loading ? (
                <div className="flex h-28 items-center justify-center text-caption text-muted-foreground">
                  <Loader2 className="w-4 h-4 animate-spin mr-1.5" />
                  {t('digitalAvatar.states.loading')}
                </div>
              ) : visibleAvatars.length === 0 ? (
                <div className="mx-4 rounded-[18px] border border-dashed border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-muted))/0.35] p-3 text-caption text-muted-foreground">
                  <p className="font-medium text-foreground">{t('digitalAvatar.states.noAvatars')}</p>
                  <p className="mt-1">{t('digitalAvatar.states.noAvatarsHint')}</p>
                </div>
              ) : (
                <div className="space-y-4 px-4">
                  <button
                    type="button"
                    className={`${BARE_BUTTON_CLASS} w-full rounded-[20px] px-3.5 py-3 text-left transition-all ${
                      globalModeActive
                        ? 'bg-[hsl(var(--ui-surface-selected))/0.64] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-strong))/0.16,0_8px_18px_hsl(var(--ui-shadow))/0.04]'
                        : 'bg-[hsl(var(--ui-surface-panel-muted))/0.32] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.12] hover:bg-[hsl(var(--ui-surface-panel-muted))/0.46] hover:shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.18]'
                    }`}
                    onClick={() => setSelectedAvatarId(null)}
                  >
                    <div className="space-y-2">
                      <div className="flex items-start justify-between gap-2">
                        <p className="pr-2 text-[12px] font-semibold tracking-[0.01em] text-foreground">
                          {t('digitalAvatar.workspace.groupModeShort', '管理组全局模式')}
                        </p>
                        <span className="inline-flex min-h-6 items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel-strong))/0.8] px-2.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
                          {t('digitalAvatar.workspace.groupModeBadge', '全局')}
                        </span>
                      </div>
                      <p className="text-[11px] leading-5 text-muted-foreground">
                        {t('digitalAvatar.workspace.groupModeHintShort', '盘点当前管理组、规划新分身与处理全局治理。')}
                      </p>
                    </div>
                  </button>
                  {avatarSections.map((section) => (
                  <div key={section.key} className="space-y-2.5">
                    <div className="flex items-center justify-between gap-2 px-1">
                      <p className="text-[13px] font-semibold leading-none tracking-[-0.01em] text-foreground">{section.title}</p>
                      <span className="inline-flex min-w-5 items-center justify-center rounded-full bg-[hsl(var(--ui-surface-panel-muted))/0.92] px-1.5 py-0.5 text-[10px] text-muted-foreground dark:bg-[hsl(var(--ui-surface-panel-muted))/0.72]">
                        {section.items.length}
                      </span>
                    </div>
                    <div className="space-y-1.5 pt-0.5">
                      {section.items.map(avatar => {
                        const selected = avatar.id === selectedAvatarId;
                        const status = normalizeAvatarStatus(avatar);
                        const projection = avatarProjectionMap[avatar.id];
                        const pendingCount = getAvatarProjectionPendingCount(projection);
                        const activityAt =
                          projection?.portalUpdatedAt || avatar.updatedAt || avatar.createdAt;
                        const extensionCount =
                          avatar.effectivePublicConfig?.effectiveAllowedExtensions?.length
                          ?? avatar.allowedExtensions?.length
                          ?? 0;
                        const skillCount =
                          avatar.effectivePublicConfig?.effectiveAllowedSkillIds?.length
                          ?? avatar.allowedSkillIds?.length
                          ?? 0;
                        const statusLabel =
                          status === 'published'
                            ? t('digitalAvatar.status.published', '已发布')
                            : status === 'draft'
                            ? t('digitalAvatar.status.draft', '草稿')
                            : status === 'disabled'
                            ? t('digitalAvatar.status.disabled', '已停用')
                            : status === 'archived'
                            ? t('digitalAvatar.status.archived', '已归档')
                            : t('digitalAvatar.labels.unset');
                        return (
                          <button
                            key={avatar.id}
                            className={`${BARE_BUTTON_CLASS} relative w-full rounded-[20px] px-3.5 py-3 text-left transition-all ${
                              selected
                                ? 'bg-[hsl(var(--ui-surface-selected))/0.66] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-strong))/0.16,0_8px_18px_hsl(var(--ui-shadow))/0.04]'
                                : 'bg-[hsl(var(--ui-surface-panel-muted))/0.28] text-foreground shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.1] hover:bg-[hsl(var(--ui-surface-panel-muted))/0.42] hover:shadow-[inset_0_0_0_1px_hsl(var(--ui-line-soft))/0.16]'
                            }`}
                            onClick={() => setSelectedAvatarId(avatar.id)}
                          >
                            <div className="min-w-0 space-y-2">
                                <div className="flex items-start justify-between gap-2">
                                  <p className="truncate pr-1 text-[12px] font-semibold tracking-[0.01em] text-foreground">{avatar.name}</p>
                                  <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                                  {pendingCount > 0 && (
                                    <span className="inline-flex min-w-[60px] justify-center h-6 items-center rounded-full border border-[hsl(var(--status-warning-text))/0.2] bg-status-warning-bg px-2.5 text-[10px] font-semibold leading-none tracking-[0.01em] text-status-warning-text">
                                      {t('digitalAvatar.labels.pendingCount', '待处理')}
                                      {' '}
                                      {pendingCount}
                                    </span>
                                  )}
                                  <span className={`inline-flex min-w-[52px] justify-center h-6 items-center rounded-full px-2.5 text-[10px] font-semibold leading-none tracking-[0.01em] ${avatarStatusBadgeClass(status)}`}>
                                    {statusLabel}
                                  </span>
                                </div>
                                </div>
                                <div className="text-[10px] leading-5 text-muted-foreground">
                                  {t('digitalAvatar.list.capabilityCounts', '扩展 {{extensions}} · 技能 {{skills}}', {
                                    extensions: extensionCount,
                                    skills: skillCount,
                                  })}
                                </div>
                                <div
                                  className="flex items-center gap-2 text-[10px] text-muted-foreground/80"
                                  title={activityAt ? formatDateTime(activityAt) : undefined}
                                >
                                  <span>
                                    {t('digitalAvatar.list.lastActivity', '最近活动')}
                                    {' · '}
                                    {activityAt ? formatRelativeTime(activityAt) : t('digitalAvatar.labels.unset')}
                                  </span>
                                </div>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))}
                </div>
              )}
            </CardContent>
          </Card>
          )}

          <div className="min-h-0">
            <Card className={`${WORKSPACE_PANEL_CLASS} min-h-0 h-full overflow-hidden flex flex-col transition-[margin] duration-200`}>
            {!focusMode && !isCompactInspectorLayout && (
            <CardHeader className="bg-[hsl(var(--ui-surface-panel-muted))/0.18] px-4 py-2.5 dark:bg-[hsl(var(--ui-surface-panel-muted))/0.2]">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="min-w-0 flex flex-wrap items-center gap-2 text-[11px] leading-5">
                  <CardTitle className="text-[13px] tracking-[-0.01em] text-foreground">
                    {t('digitalAvatar.workspace.focusTitle', '管理 Agent 对话')}
                  </CardTitle>
                  <span className="text-border/70">·</span>
                  {selectedAvatarDisplay ? (
                    <>
                        <span className="rounded-full border border-[hsl(var(--ui-line-soft))/0.68] bg-[hsl(var(--ui-surface-panel-strong))/0.86] px-2.5 py-1 text-[10px] font-semibold tracking-[0.02em] text-foreground">
                          {selectedAvatarDisplay.name}
                        </span>
                        {selectedAvatarDisplay.slug ? (
                          <span className="text-[10px] tracking-[0.06em] text-muted-foreground/80">/p/{selectedAvatarDisplay.slug}</span>
                        ) : null}
                    </>
                  ) : (
                    <>
                      <span className="text-muted-foreground">{t('digitalAvatar.workspace.groupModeShort', '管理组全局模式')}</span>
                      <button
                        type="button"
                        className={`${INSPECTOR_ACTION_LINK_CLASS} text-[11px]`}
                        onClick={() => setSelectedAvatarId(null)}
                      >
                        {t('digitalAvatar.workspace.groupModeBadge', '全局')}
                      </button>
                    </>
                  )}
                </div>
                <div className="flex shrink-0 flex-wrap items-center gap-1 rounded-full bg-background/62 p-1">
                    <Button variant="ghost" size="sm" className="h-7 rounded-full border-0 bg-transparent px-2.5 text-[10px] text-muted-foreground shadow-none hover:bg-muted/60 hover:text-foreground" onClick={toggleInspectorPanel}>
                    {inspectorOpen
                      ? t('digitalAvatar.actions.hideConsole', '收起控制台')
                      : `${t('digitalAvatar.actions.showConsole', '打开控制台')} · ${t(`digitalAvatar.inspector.${inspectorTab}` as const, inspectorTab)}`}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-7 rounded-full border-0 bg-transparent px-2.5 text-[10px] text-muted-foreground shadow-none hover:bg-[hsl(var(--ui-surface-panel-muted))/0.6] hover:text-foreground"
                    onClick={() => setTab('guide')}
                  >
                    {t('digitalAvatar.tabs.guide', '使用指南')}
                  </Button>
                  <Button variant="ghost" size="sm" className="h-7 rounded-full border-0 bg-transparent px-2.5 text-[10px] text-muted-foreground shadow-none hover:bg-[hsl(var(--ui-surface-panel-muted))/0.6] hover:text-foreground" onClick={toggleFocusMode}>
                    {t('digitalAvatar.actions.focusConversation', '专注对话')}
                  </Button>
                </div>
              </div>
            </CardHeader>
            )}
            <CardContent className="min-h-0 flex-1 overflow-hidden">
              {!effectiveManagerAgentId ? (
                <div className="h-full flex items-center justify-center">
                  <div className="text-center text-caption text-muted-foreground space-y-2">
                    <p>{t('digitalAvatar.states.noManagerAgent')}</p>
                    {canManage && (
                      <Button size="sm" variant="outline" onClick={openEcosystem}>
                        <ExternalLink className="w-3.5 h-3.5 mr-1" />
                        {t('digitalAvatar.actions.openEcosystem')}
                      </Button>
                    )}
                  </div>
                </div>
              ) : (
                <div className={`h-full flex flex-col ${focusMode ? 'gap-0' : 'gap-2'}`}>
                  <div className={`min-h-0 flex-1 overflow-hidden ${focusMode ? 'pt-0' : ''}`}>
                    <ChatConversation
                      sessionId={managerSessionId}
                      agentId={effectiveManagerAgentId}
                      agentName={managerConversationAgentName}
                      agent={managerAgent || undefined}
                      headerVariant={focusMode || workspaceChromeCollapsed || isCompactInspectorLayout ? 'compact' : 'default'}
                      inputQuickActionGroups={managerQuickActionGroups}
                      teamId={teamId}
                      createSession={createManagerSession}
                      onSessionCreated={onManagerSessionCreated}
                      onRuntimeEvent={handleRuntimeEvent}
                      onProcessingChange={setManagerProcessing}
                      composeRequest={managerComposeRequest}
                    />
                  </div>
                </div>
              )}
            </CardContent>
          </Card>
          </div>

          {!focusMode && inspectorOpen && (
            <>
              <button
                type="button"
                aria-label={t('digitalAvatar.actions.hideConsole', '收起控制台')}
                className="fixed inset-0 z-30 bg-[hsl(var(--ui-shadow))/0.36] backdrop-blur-[4px] lg:hidden"
                onClick={() => {
                  if (isConversationCompactLayout) {
                    setActiveMobilePanel(null);
                    return;
                  }
                  setInspectorOpen(false);
                }}
              />
              <div className="fixed inset-x-0 bottom-0 top-auto z-40 h-[min(84vh,760px)] w-full px-2 sm:px-3 lg:relative lg:inset-auto lg:z-auto lg:h-full lg:w-auto lg:min-h-0 lg:px-0">
                <div className={`flex h-full min-h-0 flex-col overflow-hidden rounded-t-[30px] rounded-b-none border border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-strong))/0.99] shadow-[0_-28px_60px_hsl(var(--ui-shadow))/0.28] dark:bg-[hsl(var(--ui-surface-panel-strong))/0.98] lg:rounded-[24px] lg:border-transparent lg:bg-transparent lg:shadow-none ${INSPECTOR_PANEL_CLASS}`}>
                  <div className="flex justify-center pb-1 pt-2.5 lg:hidden">
                    <span className="h-1.5 w-14 rounded-full bg-[hsl(var(--ui-line-soft))/0.94]" />
                  </div>
                  <div className="flex items-start justify-between gap-3 border-b border-[hsl(var(--ui-line-soft))/0.76] bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel-strong))_0%,hsl(var(--ui-surface-panel))_100%)] px-4 pb-3 pt-3 dark:bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel-strong))_0%,hsl(var(--ui-surface-panel))_100%)] lg:border-b-0 lg:bg-[hsl(var(--ui-surface-panel-muted))/0.92] lg:px-4 lg:py-3 dark:lg:bg-[hsl(var(--ui-surface-panel-muted))/0.86]">
                    <div className="min-w-0">
                      <p className="text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground/82">
                        {t('digitalAvatar.actions.consoleLabel', '侧边控制台')}
                      </p>
                      <p className="truncate pt-1 text-base font-semibold tracking-[-0.02em] text-foreground lg:text-sm lg:font-medium lg:tracking-normal">
                        {t(`digitalAvatar.inspector.${inspectorTab}` as const, inspectorTab)}
                      </p>
                    </div>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 rounded-full border border-[hsl(var(--ui-line-soft))/0.84] bg-background px-3 text-[11px] text-muted-foreground shadow-none hover:bg-background hover:text-foreground lg:rounded-none lg:border-0 lg:bg-transparent lg:px-2.5 lg:hover:bg-transparent"
                      onClick={() => {
                        if (isConversationCompactLayout) {
                          setActiveMobilePanel(null);
                          return;
                        }
                        setInspectorOpen(false);
                      }}
                    >
                      {t('digitalAvatar.actions.hideConsole', '收起控制台')}
                    </Button>
                  </div>
                  <div className="min-h-0 flex-1 overflow-hidden px-4 pb-4 pt-3">
                    <div className="min-h-0 h-full flex flex-col">
            {compactConsoleSnapshot}
            <div className="mb-4 rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-muted))/0.9] p-1.5 dark:bg-[hsl(var(--ui-surface-panel-muted))/0.86] lg:mb-3 lg:flex lg:items-center lg:gap-5 lg:rounded-none lg:border-0 lg:bg-transparent lg:p-0 lg:pb-2">
              <div className="grid w-full grid-cols-3 gap-1.5 lg:contents">
              {PRIMARY_INSPECTOR_TABS.map((value) => (
                <button
                  key={value}
                  className={`${BARE_BUTTON_CLASS} relative rounded-[14px] px-2 py-2 text-[11px] font-medium transition-all ${
                    inspectorTab === value
                      ? 'border border-primary/38 bg-primary/12 text-primary shadow-[0_10px_20px_hsl(var(--primary))/0.10] lg:border-0 lg:bg-transparent lg:text-foreground lg:shadow-none'
                      : 'border border-transparent text-muted-foreground hover:bg-background/80 hover:text-foreground lg:hover:bg-transparent'
                  }`}
                  onClick={() => openInspectorPanel(value)}
                >
                  {inspectorTab === value ? <span className="absolute inset-x-6 -bottom-1 h-0.5 rounded-full bg-primary lg:inset-x-0 lg:-bottom-[13px] lg:bg-foreground" /> : null}
                  {t(`digitalAvatar.inspector.${value}` as const, value)}
                </button>
              ))}
              </div>
            </div>
            <div className={`scrollbar-ghost min-h-0 overflow-x-hidden overflow-y-auto pr-0.5 ${isConversationCompactLayout ? 'space-y-2.5' : 'space-y-3'}`}>

            <div className={`${inspectorTab === 'overview' ? '' : 'hidden'} ${isConversationCompactLayout ? 'space-y-5' : 'space-y-8'}`}>
              <InspectorSection
                title={t('digitalAvatar.workspace.capabilityTitle')}
                description={t('digitalAvatar.workspace.capabilityScope', '能力开放范围')}
                action={
                  <button
                    type="button"
                    className={INSPECTOR_ACTION_LINK_CLASS}
                    onClick={openEcosystem}
                  >
                    <ExternalLink className="mr-1 h-3.5 w-3.5" />
                    {t('digitalAvatar.actions.openEcosystem')}
                  </button>
                }
              >
                <InspectorField
                  label={t('digitalAvatar.labels.currentAvatar', '当前分身')}
                  value={
                    <div className="space-y-1">
                      <div>{selectedAvatarDisplay?.name || t('digitalAvatar.labels.unset')}</div>
                      <div className="break-all text-[12px] font-normal text-muted-foreground">
                        {publishPath || t('digitalAvatar.labels.unset')}
                      </div>
                    </div>
                  }
                />
                <InspectorField
                  label={t('digitalAvatar.labels.managerAgent')}
                  value={getAgentName(agents, effectiveManagerAgentId, t('digitalAvatar.labels.unset'))}
                />
                <InspectorField
                  label={t('digitalAvatar.labels.runtimeServiceAgent', '服务 Agent')}
                  value={getAgentName(agents, selectedAvatarServiceAgentId, t('digitalAvatar.labels.unset'))}
                />
                <InspectorField
                  label={t('digitalAvatar.labels.documentAccess')}
                  value={formatDocumentAccessMode(selectedAvatarDocumentAccessMode, t)}
                />
                <InspectorField
                  label={t('digitalAvatar.list.lastActivity', '最近活动')}
                  value={
                    <span title={selectedAvatar?.updatedAt ? formatDateTime(selectedAvatar.updatedAt) : undefined}>
                      {selectedAvatarLastActivityLabel}
                    </span>
                  }
                />
              </InspectorSection>
              <InspectorSection
                title={t('digitalAvatar.workspace.capabilityScope', '能力开放范围')}
                description={selectedAvatarCapabilityScopeHint}
              >
                <InspectorField
                  label={t('digitalAvatar.labels.allowedExtensions')}
                  value={renderCapabilityChipList(
                    selectedAvatarEffectiveExtensionEntries,
                    t('digitalAvatar.labels.unset'),
                  )}
                  alignTop
                />
                <InspectorField
                  label={t('digitalAvatar.labels.allowedSkills')}
                  value={renderCapabilityChipList(
                    selectedAvatarEffectiveSkillEntries,
                    t('digitalAvatar.labels.unset'),
                  )}
                  alignTop
                />
              </InspectorSection>
            </div>

            <div className={`${inspectorTab === 'permissions' ? '' : 'hidden'} ${isConversationCompactLayout ? 'space-y-5' : 'space-y-8'}`}>
              <InspectorSection
                title={t('digitalAvatar.workspace.resourceAndAccess', '资源与权限')}
                description={t(
                  'digitalAvatar.workspace.documentAccessHint',
                  '访客可读写文档，写入行为受策略控制。',
                )}
              >
                <InspectorField
                  label={t('digitalAvatar.workspace.boundDocs', '绑定文档')}
                  value={
                    <div className="space-y-2.5">
                      {permissionSelectedDocuments.length > 0 ? (
                        renderRemovableCapabilityChipList(
                          permissionSelectedDocuments.map((doc) => ({
                            id: doc.id,
                            name: doc.display_name || doc.name,
                          })),
                          t('digitalAvatar.workspace.noBoundDocuments', '暂无可用文档'),
                          canManage
                            ? (docId) => {
                                setPermissionSelectedDocIds((current) => current.filter((id) => id !== docId));
                                setPermissionSelectedDocumentMap((current) => {
                                  const next = new Map(current);
                                  next.delete(docId);
                                  return next;
                                });
                              }
                            : undefined,
                        )
                      ) : (
                        <span className="text-[12px] font-normal text-muted-foreground">
                          {t('digitalAvatar.workspace.noBoundDocuments', '暂无可用文档')}
                        </span>
                      )}
                      {canManage ? (
                        <button
                          type="button"
                          className={INSPECTOR_ACTION_LINK_CLASS}
                          onClick={() => setShowPermissionDocPicker(true)}
                        >
                          {`+ ${t('common.edit', '编辑')}`}
                        </button>
                      ) : null}
                    </div>
                  }
                  alignTop
                />
                <InspectorField
                  label={t('digitalAvatar.labels.documentAccess')}
                  value={
                    canManage ? (
                      <select
                        value={permissionDocumentAccessMode}
                        onChange={(event) => setPermissionDocumentAccessMode(event.target.value as PortalDocumentAccessMode)}
                        className="h-9 w-full rounded-[12px] border border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel-strong))/0.92] px-3 text-sm text-foreground dark:bg-[hsl(var(--ui-surface-panel-strong))/0.82]"
                      >
                        <option value="read_only">{t('digitalAvatar.documentAccess.readOnly', '只读')}</option>
                        <option value="co_edit_draft">{t('digitalAvatar.documentAccess.coEditDraft', '协作草稿')}</option>
                        <option value="controlled_write">{t('digitalAvatar.documentAccess.controlledWrite', '受控写入')}</option>
                      </select>
                    ) : (
                      formatDocumentAccessMode(permissionDocumentAccessMode, t)
                    )
                  }
                  hint={t(
                    'digitalAvatar.workspace.documentAccessHint',
                    '访客可读写文档，写入行为受策略控制。',
                  )}
                  alignTop
                />
                <InspectorField
                  label={t('digitalAvatar.workspace.permissionPreviewTitle', '生效权限预览')}
                  value={
                    <div className="space-y-1 text-[12px] font-normal leading-6 text-muted-foreground">
                      {permissionPreviewDraft.map((line: string) => (
                        <p key={line}>{line}</p>
                      ))}
                    </div>
                  }
                  alignTop
                />
              </InspectorSection>
              <InspectorSection
                title={t('digitalAvatar.workspace.capabilityScope', '能力开放范围')}
                description={permissionScopeHint}
              >
                <InspectorField
                  label={t('digitalAvatar.workspace.allowedVisitorExtensions', '允许扩展（访客）')}
                  value={
                    <div className="space-y-2">
                      {renderRemovableCapabilityChipList(
                        permissionSelectedExtensionEntries,
                        t('digitalAvatar.labels.unset'),
                        canManage
                          ? (extensionId) => {
                              setPermissionExtensionsDirty(true);
                              setPermissionSelectedExtensions((current) =>
                                current.filter((item) => item !== extensionId),
                              );
                            }
                          : undefined,
                      )}
                      {canManage ? (
                        <button
                          type="button"
                          className={INSPECTOR_ACTION_LINK_CLASS}
                          onClick={() => setPermissionSelectorDialog('extensions')}
                        >
                          {`+ ${t('common.edit', '编辑')}`}
                        </button>
                      ) : null}
                    </div>
                  }
                  alignTop
                />
                <InspectorField
                  label={t('digitalAvatar.workspace.allowedVisitorSkills', '允许技能（访客）')}
                  value={
                    <div className="space-y-2">
                      {renderRemovableCapabilityChipList(
                        permissionSelectedSkillEntries,
                        t('digitalAvatar.labels.unset'),
                        canManage
                          ? (skillId) => {
                              setPermissionSkillsDirty(true);
                              setPermissionSelectedSkillIds((current) =>
                                current.filter((item) => item !== skillId),
                              );
                            }
                          : undefined,
                      )}
                      {canManage ? (
                        <button
                          type="button"
                          className={INSPECTOR_ACTION_LINK_CLASS}
                          onClick={() => setPermissionSelectorDialog('skills')}
                        >
                          {`+ ${t('common.edit', '编辑')}`}
                        </button>
                      ) : null}
                    </div>
                  }
                  alignTop
                />
                <InspectorField
                  label={t('digitalAvatar.workspace.capabilityScope', '能力开放范围')}
                  value={permissionScopeHint}
                  hint={t(
                    'digitalAvatar.workspace.capabilityScopeNote',
                    '这里调整的是访客可见的文档与能力边界；底层服务 Agent 的实际扩展、技能和提示词仍由管理 Agent 或高级配置维护。',
                  )}
                  alignTop
                />
                {canManage ? (
                  <div className="border-t border-[hsl(var(--ui-line-soft))/0.72] pt-4">
                    <Button
                      type="button"
                      size="sm"
                      className={`w-full ${AVATAR_PRIMARY_BUTTON_CLASS}`}
                      onClick={handleSaveAvatarPermissions}
                      disabled={savingPermissionConfig}
                    >
                      {savingPermissionConfig ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : null}
                      {t('digitalAvatar.actions.savePermissionConfig', '保存权限配置')}
                    </Button>
                  </div>
                ) : null}
              </InspectorSection>
            </div>

            <DocumentPicker
              teamId={teamId}
              open={showPermissionDocPicker}
              onClose={() => setShowPermissionDocPicker(false)}
              multiple
              selectedIds={permissionSelectedDocIds}
              selectedDocuments={permissionSelectedDocuments}
              onSelect={(docs) => {
                setPermissionSelectedDocIds(docs.map((doc) => doc.id));
                setPermissionSelectedDocumentMap(new Map(docs.map((doc) => [doc.id, doc])));
                setShowPermissionDocPicker(false);
              }}
            />

            <Dialog
              open={permissionSelectorDialog !== null}
              onOpenChange={(open) => {
                if (!open) setPermissionSelectorDialog(null);
              }}
            >
              <DialogContent className="max-w-lg">
                <DialogHeader>
                  <DialogTitle>
                    {permissionSelectorDialog === 'extensions'
                      ? t('digitalAvatar.labels.allowedExtensions')
                      : t('digitalAvatar.labels.allowedSkills')}
                  </DialogTitle>
                </DialogHeader>
                <div className="max-h-[60vh] space-y-2 overflow-y-auto">
                  {permissionSelectorDialog === 'extensions'
                    ? (selectedAvatarRuntimeExtensionOptions.length === 0 ? (
                        <p className="text-sm text-muted-foreground">
                          {t('digitalAvatar.workspace.noEnabledExtensions', '当前服务分身没有已启用扩展')}
                        </p>
                      ) : (
                        selectedAvatarRuntimeExtensionOptions.map((option) => {
                          const active = permissionSelectedExtensions.includes(option.id);
                          return (
                            <button
                              key={option.id}
                              type="button"
                              className={`w-full rounded-md border px-3 py-2 text-left ${
                                active
                                  ? 'border-primary/40 bg-primary/5'
                                  : 'border-border/60 bg-background hover:border-primary/25'
                              }`}
                              onClick={() => {
                                setPermissionExtensionsDirty(true);
                                setPermissionSelectedExtensions((current) => toggleSelection(current, option.id));
                              }}
                            >
                              <div className="flex items-start justify-between gap-3">
                                <div>
                                  <p className="text-sm font-medium text-foreground">{option.label}</p>
                                  {option.description ? (
                                    <p className="mt-1 text-xs text-muted-foreground">{option.description}</p>
                                  ) : null}
                                </div>
                                <Badge variant={active ? 'default' : 'outline'} className="text-[10px]">
                                  {active ? t('common.enabled', '已启用') : t('common.disabled', '未启用')}
                                </Badge>
                              </div>
                            </button>
                          );
                        })
                      ))
                    : (selectedAvatarAssignedSkillEntries.length === 0 ? (
                        <p className="text-sm text-muted-foreground">
                          {t('digitalAvatar.workspace.noEnabledSkills', '该 Agent 暂无已分配技能')}
                        </p>
                      ) : (
                        selectedAvatarAssignedSkillEntries.map((entry) => {
                          const active = permissionSelectedSkillIds.includes(entry.id);
                          return (
                            <button
                              key={entry.id}
                              type="button"
                              className={`w-full rounded-md border px-3 py-2 text-left ${
                                active
                                  ? 'border-primary/40 bg-primary/5'
                                  : 'border-border/60 bg-background hover:border-primary/25'
                              }`}
                              onClick={() => {
                                setPermissionSkillsDirty(true);
                                setPermissionSelectedSkillIds((current) => toggleSelection(current, entry.id));
                              }}
                            >
                              <div className="flex items-center justify-between gap-3">
                                <p className="text-sm font-medium text-foreground">{entry.name}</p>
                                <Badge variant={active ? 'default' : 'outline'} className="text-[10px]">
                                  {active ? t('common.enabled', '已启用') : t('common.disabled', '未启用')}
                                </Badge>
                              </div>
                            </button>
                          );
                        })
                      ))}
                </div>
                <div className="flex justify-end">
                  <Button type="button" variant="outline" onClick={() => setPermissionSelectorDialog(null)}>
                    {t('common.done', '完成')}
                  </Button>
                </div>
              </DialogContent>
            </Dialog>

            <Card className="hidden">
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <Clock3 className="w-4 h-4" />
                  {t('digitalAvatar.governance.controlTitle', '治理控制台')}
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.controlHint',
                    '把治理事项按待处理、已处理和自动治理记录分层显示，先决策，再回看自动执行轨迹。'
                  )}
                </p>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <div>
                      <p className="text-[11px] font-medium text-foreground">
                        {t('digitalAvatar.governance.quickFiltersTitle', '治理快筛')}
                      </p>
                      <p className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.governance.quickFiltersHint', '按事项类型、风险等级和关键词快速压缩当前治理视图。')}
                      </p>
                    </div>
                    {governanceFilterActive ? (
                      <button
                        type="button"
                        className="rounded border px-2 py-1 text-[10px] text-muted-foreground hover:text-foreground"
                        onClick={() => {
                          setGovernanceKindFilter('all');
                          setGovernanceRiskFilter('all');
                          setGovernanceSearch('');
                          setGovernanceManualOnly(false);
                        }}
                      >
                        {t('digitalAvatar.governance.clearQuickFilters', '清空筛选')}
                      </button>
                    ) : null}
                  </div>
                  <div className="flex flex-wrap items-center gap-1">
                    {(['all', 'capability', 'proposal', 'ticket', 'runtime'] as GovernanceKindFilter[]).map((kind) => (
                      <button
                        key={kind}
                        type="button"
                        className={`rounded border px-2 py-1 text-[10px] ${
                          governanceKindFilter === kind
                            ? 'border-primary/50 bg-primary/10 text-primary'
                            : 'border-border/60 bg-background text-muted-foreground hover:text-foreground'
                        }`}
                        onClick={() => setGovernanceKindFilter(kind)}
                      >
                        {kind === 'all'
                          ? t('digitalAvatar.timeline.filterAll', '全部')
                          : kind === 'capability'
                          ? t('digitalAvatar.governance.queueType.capability', '提权')
                          : kind === 'proposal'
                          ? t('digitalAvatar.governance.queueType.proposal', '新分身')
                          : kind === 'ticket'
                          ? t('digitalAvatar.governance.queueType.ticket', '优化')
                          : t('digitalAvatar.governance.timelineType.runtime', '运行')}
                      </button>
                    ))}
                    <button
                      type="button"
                      className={`rounded border px-2 py-1 text-[10px] ${
                        governanceManualOnly
                          ? 'border-primary/50 bg-primary/10 text-primary'
                          : 'border-border/60 bg-background text-muted-foreground hover:text-foreground'
                      }`}
                      onClick={() => setGovernanceManualOnly((prev) => !prev)}
                    >
                      {t('digitalAvatar.governance.manualOnlyToggle', '只看需要人工审批')}
                    </button>
                  </div>
                  <div className="flex flex-wrap items-center gap-1">
                    {(['all', 'low', 'medium', 'high'] as GovernanceRiskFilter[]).map((risk) => (
                      <button
                        key={risk}
                        type="button"
                        className={`rounded border px-2 py-1 text-[10px] ${
                          governanceRiskFilter === risk
                            ? 'border-primary/50 bg-primary/10 text-primary'
                            : 'border-border/60 bg-background text-muted-foreground hover:text-foreground'
                        }`}
                        onClick={() => setGovernanceRiskFilter(risk)}
                      >
                        {risk === 'all'
                          ? t('digitalAvatar.timeline.filterAllRisk', '全部风险')
                          : t(`digitalAvatar.timeline.risk.${risk}`, risk)}
                      </button>
                    ))}
                    <input
                      className="h-7 min-w-[112px] flex-1 rounded border bg-background px-2 text-[11px]"
                      placeholder={t('digitalAvatar.governance.quickFiltersSearch', '搜索治理标题或说明')}
                      value={governanceSearch}
                      onChange={(e) => setGovernanceSearch(e.target.value)}
                    />
                  </div>
                  <div className="flex flex-wrap items-center gap-2 text-[10px] text-muted-foreground">
                    <span>
                      {t('digitalAvatar.governance.quickFiltersPendingCount', '待处理 {{count}} / {{total}}', {
                        count: pendingGovernanceQueueItems.length,
                        total: rawPendingGovernanceQueueItems.length,
                      })}
                    </span>
                    <span>
                      {t('digitalAvatar.governance.quickFiltersResolvedCount', '已处理 {{count}} / {{total}}', {
                        count: resolvedGovernanceQueueItems.length,
                        total: rawResolvedGovernanceQueueItems.length,
                      })}
                    </span>
                    <span>
                      {t('digitalAvatar.governance.quickFiltersAutoCount', '自动治理 {{count}}', {
                        count: automatedGovernanceRows.length,
                      })}
                    </span>
                  </div>
                </div>
                <div className="space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <div>
                      <p className="text-xs font-medium text-foreground">
                        {t('digitalAvatar.governance.pendingSectionTitle', '待处理治理事项')}
                      </p>
                      <p className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.governance.pendingSectionHint', '这里只保留需要管理 Agent 或人工确认的事项。')}
                      </p>
                    </div>
                    <span className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.timelineCount', '记录 {{count}}', {
                        count: pendingGovernanceQueueItems.length,
                      })}
                    </span>
                  </div>
                  <div className="rounded-md border bg-muted/20 p-2 text-caption text-muted-foreground">
                    {t(
                      'digitalAvatar.governance.queueSummary',
                      '低风险动作会自动执行；这里仅保留需要管理 Agent 或人工确认的治理事项。'
                    )}
                  </div>
                  <div className="space-y-2 max-h-[230px] overflow-y-auto pr-1">
                    {pendingGovernanceQueueItems.length === 0 ? (
                      <p className="text-caption text-muted-foreground">
                        {t('digitalAvatar.governance.pendingSectionEmpty', '当前没有待处理治理事项')}
                      </p>
                    ) : pendingGovernanceQueueItems.map((item) => (
                      <div key={item.id} className="rounded-md border p-2">
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0">
                            <div className="flex items-center gap-1.5">
                              <span className="rounded-full border px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                {item.kind === 'capability'
                                  ? t('digitalAvatar.governance.queueType.capability', '提权')
                                  : item.kind === 'proposal'
                                  ? t('digitalAvatar.governance.queueType.proposal', '新分身')
                                  : t('digitalAvatar.governance.queueType.ticket', '优化')}
                              </span>
                              <p className="text-xs font-medium truncate">{item.title}</p>
                            </div>
                            <p className="mt-1 text-[10px] text-muted-foreground">
                              {formatDateTime(item.ts)}
                            </p>
                          </div>
                          <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                            {getGovernanceItemStatusText(t, item)}
                          </span>
                        </div>
                        {item.detail && <p className="mt-1 text-caption text-muted-foreground line-clamp-3">{item.detail}</p>}
                        {item.meta.length > 0 && (
                          <div className="mt-1.5 flex flex-wrap items-center gap-1">
                            {item.meta.map((meta) => (
                              <span key={meta} className="text-[10px] text-muted-foreground">
                                {meta}
                              </span>
                            ))}
                          </div>
                        )}
                        {canManage && item.kind === 'capability' && item.status === 'pending' && (
                          <div className="mt-2 flex flex-wrap gap-1">
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.sourceId, 'approve_direct')}>
                              {t('digitalAvatar.governance.action.approve', '通过')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.sourceId, 'approve_sandbox')}>
                              {t('digitalAvatar.governance.action.sandbox', '沙箱通过')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.sourceId, 'require_human_confirm')}>
                              {t('digitalAvatar.governance.action.human', '转人工')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => decideCapabilityRequest(item.sourceId, 'deny')}>
                              {t('digitalAvatar.governance.action.deny', '拒绝')}
                            </button>
                          </div>
                        )}
                        {canManage && item.kind === 'proposal' && (
                          <div className="mt-2 flex flex-wrap gap-1">
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.sourceId, 'pending_approval')}>
                              {t('digitalAvatar.governance.action.pending', '待审批')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.sourceId, 'approved')}>
                              {t('digitalAvatar.governance.action.approve', '通过')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.sourceId, 'pilot')}>
                              {t('digitalAvatar.governance.action.pilot', '试运行')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.sourceId, 'active')}>
                              {t('digitalAvatar.governance.action.active', '生效')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateGapProposalStatus(item.sourceId, 'rejected')}>
                              {t('digitalAvatar.governance.action.reject', '拒绝')}
                            </button>
                          </div>
                        )}
                        {canManage && item.kind === 'ticket' && (
                          <div className="mt-2 flex flex-wrap gap-1">
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.sourceId, 'approved')}>
                              <Check className="w-3 h-3 inline mr-0.5" />
                              {t('digitalAvatar.governance.action.approve', '通过')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.sourceId, 'experimenting')}>
                              {t('digitalAvatar.governance.action.experiment', '实验')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.sourceId, 'deployed')}>
                              {t('digitalAvatar.governance.action.deploy', '部署')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.sourceId, 'rolled_back')}>
                              <CircleSlash className="w-3 h-3 inline mr-0.5" />
                              {t('digitalAvatar.governance.action.rollback', '回滚')}
                            </button>
                            <button className="px-1.5 py-1 text-[10px] rounded border hover:bg-muted" onClick={() => updateOptimizationStatus(item.sourceId, 'rejected')}>
                              {t('digitalAvatar.governance.action.reject', '拒绝')}
                            </button>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              </CardContent>
            </Card>

            <Card className="hidden">
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <Clock3 className="w-4 h-4" />
                    {t('digitalAvatar.governance.timelineTitle', '治理与运行摘要')}
                  </span>
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.timelineCount', '记录 {{count}}', {
                        count: visibleGovernanceTimelineRows.length,
                      })}
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
                      onClick={() => selectedAvatar && navigate(`/teams/${teamId}/digital-avatars/${selectedAvatar.id}/timeline`)}
                      disabled={!selectedAvatar}
                    >
                      {t('digitalAvatar.timeline.openStandalone', '独立查看')}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
                      onClick={clearRuntimeSuggestions}
                      disabled={governance.runtimeLogs.length === 0}
                    >
                      {t('digitalAvatar.governance.clearRuntimeLog', '清空建议')}
                    </Button>
                  </div>
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.timelineHint',
                    '先看这条时间线快速判断最近发生了什么；需要深挖时再看完整事件流。'
                  )}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="flex items-center gap-1">
                  <button
                    className={`px-2 py-1 text-caption rounded border ${runtimeLogFilter === 'pending' ? 'bg-primary/10 border-primary/50 text-primary' : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'}`}
                    onClick={() => setRuntimeLogFilter('pending')}
                  >
                    {t('digitalAvatar.governance.runtimeFilterPending', '仅看未处理')}
                  </button>
                  <button
                    className={`px-2 py-1 text-caption rounded border ${runtimeLogFilter === 'all' ? 'bg-primary/10 border-primary/50 text-primary' : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'}`}
                    onClick={() => setRuntimeLogFilter('all')}
                  >
                    {t('digitalAvatar.governance.runtimeFilterAll', '全部')}
                  </button>
                </div>
                <div className="space-y-2 max-h-[220px] overflow-y-auto pr-1">
                  {visibleGovernanceTimelineRows.length === 0 ? (
                    <p className="text-caption text-muted-foreground">
                      {runtimeLogFilter === 'pending'
                        ? t('digitalAvatar.governance.timelineEmptyPending', '暂无待处理摘要')
                        : t('digitalAvatar.governance.timelineEmpty', '暂无摘要记录')}
                    </p>
                  ) : visibleGovernanceTimelineRows.map((item) => (
                    <div
                      key={item.id}
                      className={`rounded-md border p-2 ${
                        item.rowType === 'runtime'
                          ? 'border-status-warning/35 bg-status-warning/10'
                          : 'bg-muted/20'
                      }`}
                    >
                      <div className="flex items-center justify-between gap-2">
                        <div className="min-w-0">
                          <div className="flex items-center gap-1.5">
                            <span className="rounded-full border px-1.5 py-0.5 text-[10px] text-muted-foreground">
                              {item.rowType === 'runtime'
                                ? t('digitalAvatar.governance.timelineType.runtime', '运行')
                                : item.rowType === 'capability'
                                ? t('digitalAvatar.governance.timelineType.capability', '提权')
                                : item.rowType === 'proposal'
                                ? t('digitalAvatar.governance.timelineType.proposal', '新分身')
                                : t('digitalAvatar.governance.timelineType.ticket', '优化')}
                            </span>
                            <p className="text-xs font-medium truncate">{item.title}</p>
                          </div>
                          <p className="mt-1 text-[10px] text-muted-foreground" title={formatDateTime(item.ts)}>
                            {formatRelativeTime(item.ts)}
                          </p>
                        </div>
                        <div className="flex items-center gap-1">
                          <span className={`px-1.5 py-0.5 rounded text-[10px] border ${
                            item.rowType === 'runtime' ? runtimeStatusClass(item.status as RuntimeLogStatus) : badgeClass(item.status)
                          }`}>
                            {getGovernanceItemStatusText(t, item)}
                          </span>
                        </div>
                      </div>
                      {item.detail && <p className="mt-1 text-caption text-muted-foreground line-clamp-3">{item.detail}</p>}
                      {item.meta.length > 0 && (
                        <div className="mt-1.5 flex flex-wrap items-center gap-1">
                          {item.meta.map((meta) => (
                            <span key={meta} className="text-[10px] text-muted-foreground">
                              {meta}
                            </span>
                          ))}
                        </div>
                      )}
                      {item.rowType === 'runtime' && item.runtimeId && (
                        <div className="mt-1.5 flex items-center justify-between gap-2">
                          <span className="text-[10px] text-muted-foreground">{formatDateTime(item.ts)}</span>
                          {item.status === 'pending' ? (
                            <div className="flex flex-wrap gap-1">
                              <button
                                className="px-1.5 py-1 text-[10px] rounded border hover:bg-background"
                                onClick={() => dismissRuntimeSuggestion(item.runtimeId!)}
                              >
                                {t('common.dismiss', '忽略')}
                              </button>
                            </div>
                          ) : (
                            <button
                              className="px-1.5 py-1 text-[10px] rounded border hover:bg-background"
                              onClick={() => resetRuntimeSuggestion(item.runtimeId!)}
                            >
                              {t('digitalAvatar.governance.runtimeRestore', '恢复待处理')}
                            </button>
                          )}
                        </div>
                      )}
                      {item.rowType !== 'runtime' && (
                        <div className="mt-1.5 flex items-center justify-between gap-2">
                          <span className="text-[10px] text-muted-foreground">{formatDateTime(item.ts)}</span>
                          <span className="text-[10px] text-muted-foreground">{getGovernanceItemStatusText(t, item)}</span>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card className="hidden">
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <ShieldAlert className="w-4 h-4" />
                    {t('digitalAvatar.governance.decisionAuditTitle', '治理决策审计')}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    {t('digitalAvatar.governance.decisionAuditCount', '记录 {{count}}', {
                      count: governanceAuditRows.length,
                    })}
                  </span>
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t('digitalAvatar.governance.decisionAuditHint', '集中展示管理 Agent 与人工的审批/驳回/部署等决策记录。')}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="space-y-2 max-h-[180px] overflow-y-auto pr-1">
                  {governanceAuditRows.length === 0 ? (
                    <p className="text-caption text-muted-foreground">
                      {t('digitalAvatar.governance.decisionAuditEmpty', '暂无决策记录')}
                    </p>
                  ) : governanceAuditRows.slice(0, 6).map((item) => (
                    <div key={item.id} className="rounded-md border p-2 bg-muted/20">
                      <div className="flex items-center justify-between gap-2">
                        <p className="text-xs font-medium truncate">{item.title}</p>
                        <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                          {item.type}
                        </span>
                      </div>
                      <p className="mt-1 text-caption text-muted-foreground line-clamp-2">
                        {item.detail || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}
                      </p>
                      <div className="mt-1 flex items-center justify-between gap-2">
                        <span className="text-[10px] text-muted-foreground">
                          {formatDateTime(item.ts)}
                        </span>
                        <span className="text-[10px] text-muted-foreground">{getGovernanceItemStatusText(t, item)}</span>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card className="hidden">
              <CardHeader className="pb-2">
                <div className="space-y-2">
                  <CardTitle className="flex items-center gap-2 text-sm">
                    <Clock3 className="h-4 w-4 shrink-0" />
                    <span className="font-medium text-foreground">
                      {t('digitalAvatar.governance.runtimeEventsTitleShort', '运行日志')}
                    </span>
                    <span className="rounded-full border border-border/60 bg-muted/20 px-1.5 py-0.5 text-[10px] font-normal text-muted-foreground">
                      {t('digitalAvatar.governance.runtimeEventsTraceable', '可追溯')}
                    </span>
                  </CardTitle>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.runtimeEventsCountCompact', '共 {{count}} 条', {
                        count: persistedEvents.length,
                      })}
                    </span>
                    <div className="flex items-center gap-1">
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-6 px-2 text-[10px]"
                        onClick={loadOlderPersistedEvents}
                        disabled={!persistedEventsHasMore || persistedEventsLoadingMore || persistedEventsLoading}
                      >
                        {persistedEventsLoadingMore ? (
                          <Loader2 className="w-3 h-3 animate-spin" />
                        ) : (
                          t('digitalAvatar.governance.runtimeEventsLoadOlderShort', '更早')
                        )}
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-6 w-6 px-0"
                        onClick={refreshPersistedEvents}
                        disabled={persistedEventsLoading}
                      >
                        {persistedEventsLoading ? (
                          <Loader2 className="w-3 h-3 animate-spin" />
                        ) : (
                          <RefreshCw className="w-3 h-3" />
                        )}
                      </Button>
                    </div>
                  </div>
                </div>
                <p className="text-caption text-muted-foreground">
                  {t(
                    'digitalAvatar.governance.runtimeEventsHint',
                    '记录管理 Agent 全量执行事件（状态/思考/工具/结果/完成），支持分类筛选与追溯排查。'
                  )}
                </p>
              </CardHeader>
              <CardContent className="space-y-2">
                <div className="flex flex-wrap items-center gap-1">
                  {(['all', 'error', 'tool', 'thinking', 'status'] as PersistedEventFilter[]).map((kind) => (
                    <button
                      key={kind}
                      className={`px-2 py-1 text-caption rounded border ${
                        persistedEventFilter === kind
                          ? 'bg-primary/10 border-primary/50 text-primary'
                          : 'bg-background border-border/60 text-muted-foreground hover:text-foreground'
                      }`}
                      onClick={() => setPersistedEventFilter(kind)}
                    >
                      {t(`digitalAvatar.governance.runtimeEventsFilter.${kind}`, kind)}
                    </button>
                  ))}
                  <input
                    className="h-7 min-w-[108px] flex-1 rounded border bg-background px-2 text-[11px]"
                    placeholder={t('digitalAvatar.governance.runtimeEventsSearch', '搜索事件内容')}
                    value={persistedEventSearch}
                    onChange={(e) => setPersistedEventSearch(e.target.value)}
                  />
                </div>
                {persistedEventsError && (
                  <div className="rounded border border-status-error/40 bg-status-error/10 px-2 py-1 text-[10px] text-status-error-text">
                    {persistedEventsError}
                  </div>
                )}
                <div className="space-y-2 max-h-[260px] overflow-y-auto pr-1">
                  {visiblePersistedEvents.length === 0 ? (
                    <p className="text-caption text-muted-foreground">
                      {t('digitalAvatar.governance.runtimeEventsEmpty', '暂无可展示事件')}
                    </p>
                  ) : visiblePersistedEvents.map((event) => {
                    const severity = eventSeverity(event);
                    return (
                      <div key={event.displayKey} className={`rounded-md border p-2 ${severityClass(severity)}`}>
                        <div className="flex items-start justify-between gap-2">
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-1.5">
                              <p className="text-[11px] font-medium text-foreground">
                                #{event.event_id} · {event.event_type}
                              </p>
                              {event.mergedCount && event.mergedCount > 1 ? (
                                <span className="rounded-full border border-border/60 bg-background px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                  {t('digitalAvatar.governance.runtimeEventsMerged', '合并 {{count}} 条', {
                                    count: event.mergedCount,
                                  })}
                                </span>
                              ) : null}
                            </div>
                            <p className="mt-1 text-[10px] text-muted-foreground break-all">
                              {event.run_id || 'run:unknown'} · {new Date(event.created_at).toLocaleString()}
                            </p>
                          </div>
                          <span className={`shrink-0 px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(severity === 'error' ? 'rejected' : severity === 'warn' ? 'pending' : 'approved')}`}>
                            {eventTypeBadge(event.event_type)}
                          </span>
                        </div>
                        <p className="mt-2 text-[11px] leading-5 text-muted-foreground whitespace-pre-wrap break-normal">
                          {eventSummary(event) || t('digitalAvatar.governance.runtimeEventsNoDetail', '无详细内容')}
                        </p>
                      </div>
                    );
                  })}
                </div>
                {!persistedEventsHasMore && persistedEvents.length > 0 && (
                  <p className="text-[10px] text-muted-foreground text-center">
                    {t('digitalAvatar.governance.runtimeEventsNoOlder', '已加载到最早事件')}
                  </p>
                )}
              </CardContent>
            </Card>

            <Card className="hidden">
              <CardContent className="py-3">
                <div className="mb-2 rounded-md border bg-muted/20 px-2 py-1.5 text-[10px] text-muted-foreground">
                  {t('digitalAvatar.governance.autoEngineSummary', '自动治理已启用；同类缺口累计 {{count}} 次自动生成新增分身提案。', {
                    count: governanceConfig.autoProposalTriggerCount,
                  })}
                </div>
                <div className="mb-2 rounded-md border bg-muted/20 p-2">
                  <div className="flex items-center justify-between gap-2">
                    <div className="min-w-0">
                      <p className="text-[11px] font-medium text-foreground">
                        {t('digitalAvatar.governance.autoProposalThresholdLabel', '自动提案触发阈值')}
                      </p>
                      <p className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.governance.autoProposalThresholdHint', '同类缺口累计达到该次数后，自动生成新增分身提案（1-10）。')}
                      </p>
                    </div>
                    <div className="flex items-center gap-1.5 shrink-0">
                      <input
                        type="number"
                        min={1}
                        max={10}
                        className="h-7 w-16 rounded border bg-background px-2 text-[11px]"
                        value={autoProposalTriggerCountDraft}
                        onChange={(e) => setAutoProposalTriggerCountDraft(Number(e.target.value || 0))}
                        disabled={!canManage || savingAutomationConfig}
                      />
                      {canManage && (
                        <Button
                          size="sm"
                          variant="outline"
                          className="h-7 px-2 text-[11px]"
                          onClick={saveAutomationConfig}
                          disabled={savingAutomationConfig}
                        >
                          {savingAutomationConfig ? <Loader2 className="w-3 h-3 animate-spin" /> : t('common.save', '保存')}
                        </Button>
                      )}
                    </div>
                  </div>
                </div>
                <div className="grid grid-cols-3 gap-2 text-center">
                  <div className="rounded-md border p-2">
                    <p className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.stats.pendingReq', '待处理请求')}
                    </p>
                    <p className="text-sm font-semibold">{governanceStats.pendingCapability}</p>
                  </div>
                  <div className="rounded-md border p-2">
                    <p className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.stats.pendingProposal', '待审批提案')}
                    </p>
                    <p className="text-sm font-semibold">{governanceStats.pendingProposals}</p>
                  </div>
                  <div className="rounded-md border p-2">
                    <p className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.stats.pendingTicket', '待处理工单')}
                    </p>
                    <p className="text-sm font-semibold">{governanceStats.pendingTickets}</p>
                  </div>
                </div>
                <p className="mt-2 text-caption text-muted-foreground">
                  {managerProcessing
                    ? t('digitalAvatar.governance.managerWorking', '管理 Agent 正在运行，可持续产生优化建议。')
                    : t('digitalAvatar.governance.managerIdle', '管理 Agent 空闲，可发起新一轮能力评估。')}
                </p>
                <div className="mt-4 space-y-4">
                  <div className="space-y-2">
                    <div className="flex items-center justify-between gap-2">
                      <div>
                        <p className="text-xs font-medium text-foreground">
                          {t('digitalAvatar.governance.resolvedSectionTitle', '已处理事项')}
                        </p>
                        <p className="text-[10px] text-muted-foreground">
                          {t('digitalAvatar.governance.resolvedSectionHint', '保留最近已审批、已部署或已拒绝的治理结果，方便快速复盘。')}
                        </p>
                      </div>
                      <span className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.governance.timelineCount', '记录 {{count}}', {
                          count: resolvedGovernanceQueueItems.length,
                        })}
                      </span>
                    </div>
                    <div className="space-y-2 max-h-[180px] overflow-y-auto pr-1">
                      {resolvedGovernanceQueueItems.length === 0 ? (
                        <p className="text-caption text-muted-foreground">
                          {t('digitalAvatar.governance.resolvedSectionEmpty', '暂无已处理治理事项')}
                        </p>
                      ) : resolvedGovernanceQueueItems.slice(0, 8).map((item) => (
                        <div key={item.id} className="rounded-md border bg-muted/20 p-2">
                          <div className="flex items-center justify-between gap-2">
                            <div className="min-w-0">
                              <div className="flex items-center gap-1.5">
                                <span className="rounded-full border px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                  {item.kind === 'capability'
                                    ? t('digitalAvatar.governance.queueType.capability', '提权')
                                    : item.kind === 'proposal'
                                    ? t('digitalAvatar.governance.queueType.proposal', '新分身')
                                    : t('digitalAvatar.governance.queueType.ticket', '优化')}
                                </span>
                                <p className="text-xs font-medium truncate">{item.title}</p>
                              </div>
                              <p className="mt-1 text-[10px] text-muted-foreground">{formatDateTime(item.ts)}</p>
                            </div>
                            <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(item.status)}`}>
                              {getGovernanceItemStatusText(t, item)}
                            </span>
                          </div>
                          {item.detail ? <p className="mt-1 text-caption text-muted-foreground line-clamp-2">{item.detail}</p> : null}
                        </div>
                      ))}
                    </div>
                  </div>

                  <div className="space-y-2">
                    <div className="flex items-center justify-between gap-2">
                      <div>
                        <p className="text-xs font-medium text-foreground">
                          {t('digitalAvatar.governance.autoSectionTitle', '自动治理记录')}
                        </p>
                        <p className="text-[10px] text-muted-foreground">
                          {t('digitalAvatar.governance.autoSectionHint', '展示自动发现的问题、自动转单痕迹与人工回滚入口。')}
                        </p>
                      </div>
                      <span className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.governance.timelineCount', '记录 {{count}}', {
                          count: automatedGovernanceRows.length,
                        })}
                      </span>
                    </div>
                    <div className="space-y-2 max-h-[200px] overflow-y-auto pr-1">
                      {automatedGovernanceRows.length === 0 ? (
                        <p className="text-caption text-muted-foreground">
                          {t('digitalAvatar.governance.autoSectionEmpty', '暂无自动治理记录')}
                        </p>
                      ) : automatedGovernanceRows.map((item) => (
                        <div key={item.id} className="rounded-md border border-status-warning/35 bg-status-warning/10 p-2">
                          <div className="flex items-center justify-between gap-2">
                            <div className="min-w-0">
                              <p className="text-xs font-medium truncate">{item.title}</p>
                              <p className="mt-1 text-[10px] text-muted-foreground">{formatDateTime(item.ts)}</p>
                            </div>
                            <span className={`px-1.5 py-0.5 rounded text-[10px] border ${runtimeStatusClass(item.status as RuntimeLogStatus)}`}>
                              {getRuntimeStatusText(t, item.status)}
                            </span>
                          </div>
                          {item.detail ? <p className="mt-1 text-caption text-muted-foreground line-clamp-3">{item.detail}</p> : null}
                          {item.meta.length > 0 ? (
                            <div className="mt-1.5 flex flex-wrap items-center gap-1">
                              {item.meta.map((meta) => (
                                <span key={meta} className="text-[10px] text-muted-foreground">
                                  {meta}
                                </span>
                              ))}
                            </div>
                          ) : null}
                          {item.runtimeId ? (
                            <div className="mt-2 flex flex-wrap gap-1">
                              {item.status === 'pending' ? (
                                <button
                                  className="px-1.5 py-1 text-[10px] rounded border hover:bg-background"
                                  onClick={() => dismissRuntimeSuggestion(item.runtimeId!)}
                                >
                                  {t('common.dismiss', '忽略')}
                                </button>
                              ) : (
                                <button
                                  className="px-1.5 py-1 text-[10px] rounded border hover:bg-background"
                                  onClick={() => resetRuntimeSuggestion(item.runtimeId!)}
                                >
                                  {t('digitalAvatar.governance.runtimeRestore', '恢复待处理')}
                                </button>
                              )}
                            </div>
                          ) : null}
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              </CardContent>
            </Card>
            <div className={`${inspectorTab === 'publish' ? '' : 'hidden'} ${isConversationCompactLayout ? 'space-y-5' : 'space-y-8'}`}>
                {!selectedAvatarDisplay ? (
                  <p className="text-[12px] leading-6 text-muted-foreground">{t('digitalAvatar.states.noAvatarSelected', '未选择分身')}</p>
                ) : (
                    <>
                      <InspectorSection
                        title={t('digitalAvatar.workspace.publishTitle', '发布视图')}
                        description={
                          selectedAvatarStatus === 'published'
                            ? selectedAvatarEffectivePublicConfig?.publicAccessEnabled
                              ? t('digitalAvatar.workspace.publishControlPublishedHint', '当前已发布，访客可通过正式入口访问。')
                              : t('digitalAvatar.workspace.publishControlPreviewOnlyHint', '当前处于已发布但仅管理预览状态，不会对外暴露正式访客页。')
                            : selectedAvatarStatus === 'archived'
                              ? t('digitalAvatar.workspace.publishControlArchivedHint', '当前已归档，重新发布后才会恢复访客访问。')
                              : t('digitalAvatar.workspace.publishControlDraftHint', '草稿状态下，访客页不可访问，仅可通过管理预览或测试入口验证。')
                        }
                        action={
                          canManage ? (
                            <Button
                              size="sm"
                              className={AVATAR_PRIMARY_BUTTON_CLASS}
                              onClick={handleToggleAvatarPublish}
                              disabled={publishingAvatar}
                            >
                              {selectedAvatarStatus === 'published'
                                ? t('digitalAvatar.actions.unpublishAvatar', '停止对外服务')
                                : t('digitalAvatar.actions.publishAvatar', '发布分身')}
                            </Button>
                          ) : null
                        }
                      >
                        <InspectorField
                          label={t('digitalAvatar.workspace.summaryStatus', '当前状态')}
                          value={selectedAvatarStatusLabel}
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.summaryPublish', '发布地址')}
                          value={selectedAvatarPublicUrl || (
                            selectedAvatarStatus === 'published'
                              ? t('digitalAvatar.workspace.previewOnly', '仅管理预览')
                              : t('digitalAvatar.workspace.unpublished', '未发布')
                          )}
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.summaryType', '分身类型')}
                          value={<AvatarTypeBadge type={selectedAvatarType} />}
                        />
                        <InspectorField
                          label={t('common.description', '描述')}
                          value={selectedAvatarDisplay?.description || t('digitalAvatar.labels.unset')}
                          alignTop
                        />
                      </InspectorSection>
                      <InspectorSection
                        title={t('digitalAvatar.workspace.publicConfigTitle', '对外配置与生效')}
                        description={t(
                          'digitalAvatar.workspace.publishMode.compareHint',
                          '访客页用于正式对外交付，管理预览用于内部验收，测试入口用于联调排查。'
                        )}
                        action={
                          <button
                            type="button"
                            className={INSPECTOR_ACTION_LINK_CLASS}
                            onClick={() => setPublishModeGuideOpen(true)}
                          >
                            {t('digitalAvatar.workspace.publishMode.openGuide', '查看说明')}
                          </button>
                        }
                      >
                        <InspectorField
                          label={t('digitalAvatar.workspace.outputFormLabel', '配置输出形态')}
                          value={selectedAvatarOutputFormLabel}
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.effectiveExposureLabel', '生效对外曝光')}
                          value={selectedAvatarExposureLabel}
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.publishMode.currentMode', '当前视角')}
                          value={
                            <div className="flex flex-wrap gap-4">
                              {availablePublishModes.map((mode) => {
                                const active = publishViewMode === mode;
                                const label =
                                  mode === 'visitor'
                                    ? t('digitalAvatar.workspace.publishMode.visitorTab', '访客视角')
                                    : mode === 'preview'
                                      ? t('digitalAvatar.workspace.publishMode.previewTab', '管理预览')
                                      : t('digitalAvatar.workspace.publishMode.testTab', '测试入口');
                                return (
                                  <button
                                    key={mode}
                                    type="button"
                                    className={`${BARE_BUTTON_CLASS} relative pb-1 text-[12px] transition-colors ${
                                      active ? 'font-semibold text-foreground' : 'text-muted-foreground hover:text-foreground'
                                    }`}
                                    onClick={() => setPublishViewMode(mode)}
                                  >
                                    {label}
                                    {active ? <span className="absolute inset-x-0 bottom-0 h-px bg-foreground" /> : null}
                                  </button>
                                );
                              })}
                            </div>
                          }
                          alignTop
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.publishMode.visitorAddress', '访客入口')}
                          value={activePublishUrl || t('digitalAvatar.labels.unset')}
                          hint={publishModeDescription.title}
                          alignTop
                        />
                      </InspectorSection>
                      <InspectorSection
                        title={t('digitalAvatar.workspace.publicNarrativeTitle', '公开页顶部叙事')}
                        description={
                          selectedAvatarNarrativeConfigured
                            ? t(
                                'digitalAvatar.workspace.publicNarrativeConfiguredHint',
                                '已配置对外页面顶部说明，可向访客解释这个分身为什么存在、适合处理什么以及如何开始。'
                              )
                            : t(
                                'digitalAvatar.workspace.publicNarrativeEmptyHint',
                                '还未配置顶部叙事，建议补充一段用户向说明，帮助访客快速理解这个分身的定位。'
                              )
                        }
                        action={
                          canManage ? (
                            <button
                              type="button"
                              className={INSPECTOR_ACTION_LINK_CLASS}
                              onClick={() => setPublicNarrativeDialogOpen(true)}
                            >
                              {t('digitalAvatar.workspace.editPublicNarrative', '编辑叙事')}
                            </button>
                          ) : null
                        }
                      >
                        <InspectorField
                          label={t('digitalAvatar.workspace.publicNarrativeSummaryLabel', '当前摘要')}
                          value={publishHeroIntro.trim() || t('digitalAvatar.labels.unset')}
                          alignTop
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.publicNarrativeUseCasesLabel', '典型任务')}
                          value={
                            selectedAvatarNarrativeUseCases.length > 0 ? (
                              <ul className="space-y-1.5 pl-4 text-[13px] font-medium leading-6 text-foreground">
                                {selectedAvatarNarrativeUseCases.slice(0, 4).map((item) => (
                                  <li key={item} className="list-disc">
                                    {item}
                                  </li>
                                ))}
                              </ul>
                            ) : (
                              t('digitalAvatar.workspace.publicNarrativeNoUseCases', '未设置典型任务')
                            )
                          }
                          alignTop
                        />
                      </InspectorSection>
                      <InspectorSection
                        title={t('digitalAvatar.workspace.publicConfigTitle', '对外配置与生效')}
                        description={t('digitalAvatar.workspace.publishControlDescription', '面向访客的入口信息、边界与可见内容。')}
                      >
                        <InspectorField
                          label={t('digitalAvatar.workspace.chatWidgetConfigLabel', '聊天挂件配置')}
                          value={
                            <div className="flex items-center justify-between gap-3">
                              <span>
                                {selectedAvatarShowChatWidget
                                  ? t('common.enabled', '已开启')
                                  : t('common.disabled', '已关闭')}
                              </span>
                              {canManage ? (
                                <button
                                  type="button"
                                  role="switch"
                                  aria-checked={selectedAvatarShowChatWidget}
                                  aria-label={t('digitalAvatar.workspace.chatWidgetConfigLabel', '聊天挂件配置')}
                                  disabled={updatingPublicConfig}
                                  onClick={handleToggleChatWidget}
                                  className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border transition-colors ${
                                    selectedAvatarShowChatWidget
                                      ? 'border-primary/50 bg-primary/20'
                                      : 'border-border/70 bg-muted/60'
                                  } ${updatingPublicConfig ? 'cursor-not-allowed opacity-60' : 'cursor-pointer'}`}
                                >
                                  <span
                                    className={`inline-block h-5 w-5 rounded-full bg-background shadow-sm transition-transform ${
                                      selectedAvatarShowChatWidget ? 'translate-x-5' : 'translate-x-0.5'
                                    }`}
                                  />
                                </button>
                              ) : null}
                            </div>
                          }
                          hint={selectedAvatarChatWidgetEffectLabel}
                          alignTop
                        />
                        <InspectorField
                          label={t('digitalAvatar.workspace.boundDocumentsVisibilityLabel', '平台资料展示')}
                          value={
                            <div className="flex items-center justify-between gap-3">
                              <span>
                                {selectedAvatarShowBoundDocuments
                                  ? t('digitalAvatar.workspace.boundDocumentsVisibleState', '对外可见')
                                  : t('digitalAvatar.workspace.boundDocumentsHiddenState', '仅供分身内部使用')}
                              </span>
                              {canManage ? (
                                <button
                                  type="button"
                                  role="switch"
                                  aria-checked={selectedAvatarShowBoundDocuments}
                                  aria-label={t('digitalAvatar.workspace.boundDocumentsVisibilityLabel', '平台资料展示')}
                                  disabled={updatingPublicConfig}
                                  onClick={handleToggleBoundDocuments}
                                  className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full border transition-colors ${
                                    selectedAvatarShowBoundDocuments
                                      ? 'border-primary/50 bg-primary/20'
                                      : 'border-border/70 bg-muted/60'
                                  } ${updatingPublicConfig ? 'cursor-not-allowed opacity-60' : 'cursor-pointer'}`}
                                >
                                  <span
                                    className={`inline-block h-5 w-5 rounded-full bg-background shadow-sm transition-transform ${
                                      selectedAvatarShowBoundDocuments ? 'translate-x-5' : 'translate-x-0.5'
                                    }`}
                                  />
                                </button>
                              ) : null}
                            </div>
                          }
                          hint={
                            selectedAvatarShowBoundDocuments
                              ? t('digitalAvatar.workspace.boundDocumentsVisibleHint', '访客可以在公开页中直接看到绑定的平台资料。')
                              : t('digitalAvatar.workspace.boundDocumentsHiddenHint', '绑定文档仍会给分身内部使用，但访客端不会直接展示明细。')
                          }
                          alignTop
                        />
                        <div className="flex flex-col gap-2.5 border-t border-[hsl(var(--ui-line-soft))/0.72] pt-4">
                          <button
                            type="button"
                            className={INSPECTOR_ACTION_LINK_CLASS}
                            onClick={() => activePublishUrl && window.open(activePublishUrl, '_blank', 'noopener,noreferrer')}
                            disabled={!activePublishUrl}
                          >
                            <ExternalLink className="mr-1 h-3.5 w-3.5" />
                            {publishViewMode === 'visitor'
                              ? t('digitalAvatar.workspace.openPublicPage', '打开访客页')
                              : publishViewMode === 'preview'
                                ? t('digitalAvatar.workspace.openPreviewPage', '打开管理预览')
                                : t('digitalAvatar.workspace.openTestPage', '打开测试入口')}
                          </button>
                          {selectedAvatarPreviewUrl && publishViewMode !== 'preview' ? (
                            <button
                              type="button"
                              className={INSPECTOR_ACTION_LINK_CLASS}
                              onClick={() => window.open(selectedAvatarPreviewUrl, '_blank', 'noopener,noreferrer')}
                            >
                              <ExternalLink className="mr-1 h-3.5 w-3.5" />
                              {t('digitalAvatar.workspace.openPreviewPage', '打开管理预览')}
                            </button>
                          ) : null}
                          <button
                            type="button"
                            className={INSPECTOR_ACTION_LINK_CLASS}
                            onClick={openEcosystem}
                          >
                            <ExternalLink className="mr-1 h-3.5 w-3.5" />
                            {t('digitalAvatar.actions.openEcosystem')}
                          </button>
                        </div>
                      </InspectorSection>
                  </>
                )}
            </div>
                    </div>
                    </div>
                  </div>
                </div>
              </div>
            </>
          )}
        </div>
        )}
      </div>
      </>

      <Dialog open={publicNarrativeDialogOpen} onOpenChange={setPublicNarrativeDialogOpen}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t('digitalAvatar.workspace.publicNarrativeTitle', '公开页顶部叙事')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <p className="text-sm leading-6 text-muted-foreground">
              {t(
                'digitalAvatar.workspace.publicNarrativeDialogHint',
                '这里填写的是公开页顶部给访客看的说明，重点解释这个分身为什么存在、适合处理什么，以及用户该如何开始。',
              )}
            </p>
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('digitalAvatar.workspace.publicNarrativeIntroLabel', '顶部主叙事')}
              </label>
              <Textarea
                rows={4}
                value={publishHeroIntro}
                onChange={(event) => setPublishHeroIntro(event.target.value)}
                placeholder={t(
                  'digitalAvatar.workspace.publicNarrativeIntroPlaceholder',
                  '例如：这是一个面向客户支持的服务分身，专门帮助用户基于产品资料快速定位问题、整理答案并给出下一步建议。',
                )}
                disabled={!canManage}
              />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium text-foreground">
                {t('digitalAvatar.workspace.publicNarrativeUseCasesLabel', '典型任务（每行一条）')}
              </label>
              <Textarea
                rows={5}
                value={publishHeroUseCasesText}
                onChange={(event) => setPublishHeroUseCasesText(event.target.value)}
                placeholder={t(
                  'digitalAvatar.workspace.publicNarrativeUseCasesPlaceholder',
                  '回答产品使用问题\n根据资料整理计划\n继续处理指定文档',
                )}
                disabled={!canManage}
              />
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <label className="text-sm font-medium text-foreground">
                  {t('digitalAvatar.workspace.publicNarrativeWorkingStyleLabel', '处理方式说明')}
                </label>
                <Textarea
                  rows={4}
                  value={publishHeroWorkingStyle}
                  onChange={(event) => setPublishHeroWorkingStyle(event.target.value)}
                  placeholder={t(
                    'digitalAvatar.workspace.publicNarrativeWorkingStylePlaceholder',
                    '例如：我会先基于当前开放资料处理；超出范围时，会继续交给管理 Agent 判断。',
                  )}
                  disabled={!canManage}
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium text-foreground">
                  {t('digitalAvatar.workspace.publicNarrativeCtaHintLabel', '开始提示')}
                </label>
                <Textarea
                  rows={4}
                  value={publishHeroCtaHint}
                  onChange={(event) => setPublishHeroCtaHint(event.target.value)}
                  placeholder={t(
                    'digitalAvatar.workspace.publicNarrativeCtaHintPlaceholder',
                    '例如：直接在对话频道描述问题；如果需要我结合资料处理，先去资料频道选中目标文档。',
                  )}
                  disabled={!canManage}
                />
              </div>
            </div>
            <div className="flex flex-wrap justify-end gap-2">
              <Button variant="outline" onClick={() => setPublicNarrativeDialogOpen(false)}>
                {t('common.cancel', '取消')}
              </Button>
              {canManage ? (
                <Button className={AVATAR_PRIMARY_BUTTON_CLASS} onClick={handleSavePublicNarrative} disabled={savingNarrativeConfig}>
                  {savingNarrativeConfig ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : null}
                  {t('common.save', '保存')}
                </Button>
              ) : null}
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <Dialog open={publishModeGuideOpen} onOpenChange={setPublishModeGuideOpen}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>{t('digitalAvatar.workspace.publishMode.compareTitle', '视角切换说明')}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <p className="text-sm leading-6 text-muted-foreground">
              {t('digitalAvatar.workspace.publishMode.compareHint', '访客页用于正式对外交付，管理预览用于内部验收，测试入口用于联调排查。')}
            </p>
            <div className="grid gap-3 sm:grid-cols-3">
              {availablePublishModes.map((mode) => {
                const title = mode === 'visitor'
                  ? t('digitalAvatar.workspace.publishMode.visitorTab', '访客视角')
                  : mode === 'preview'
                  ? t('digitalAvatar.workspace.publishMode.previewTab', '管理预览')
                  : t('digitalAvatar.workspace.publishMode.testTab', '测试入口');
                const desc = mode === 'visitor'
                  ? t('digitalAvatar.workspace.publishMode.visitorDesc', '这是外部用户最终看到的数字分身页面，重点是能力说明、边界提示和对话入口。')
                  : mode === 'preview'
                  ? t('digitalAvatar.workspace.publishMode.previewDesc', '用于内部检查页面内容、权限边界和对话入口是否按预期展示。')
                  : t('digitalAvatar.workspace.publishMode.testDesc', '用于联调、网络验证或内网快速访问，不代表正式访客入口。');
                return (
                  <div
                    key={`publish-guide-${mode}`}
                    className={`rounded-md border p-3 ${
                      publishViewMode === mode
                        ? 'border-primary/50 bg-primary/5'
                        : 'border-border/60 bg-muted/10'
                    }`}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <p className="text-sm font-medium text-foreground">{title}</p>
                      {publishViewMode === mode ? (
                        <span className="rounded-full border border-primary/40 bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">
                          {t('digitalAvatar.workspace.publishMode.currentMode', '当前')}
                        </span>
                      ) : null}
                    </div>
                    <p className="mt-2 text-xs leading-6 text-muted-foreground">{desc}</p>
                  </div>
                );
              })}
            </div>
            <div className="rounded-md border bg-muted/10 p-3">
              <p className="text-sm font-medium text-foreground">{publishModeDescription.title}</p>
              <p className="mt-2 text-xs leading-6 text-muted-foreground">{publishModeDescription.description}</p>
              <div className="mt-3 space-y-2 text-xs leading-6 text-muted-foreground">
                {publishModeDescription.bullets.map((bullet) => (
                  <p key={bullet}>{bullet}</p>
                ))}
              </div>
            </div>
            <div className="flex justify-end">
              <Button variant="outline" onClick={() => setPublishModeGuideOpen(false)}>
                {t('common.close', '关闭')}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      <CreateAvatarDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        teamId={teamId}
        managerAgentId={selectedManagerGroupId}
        managerAgentName={getAgentName(agents, selectedManagerGroupId, t('digitalAvatar.labels.unset'))}
        onCreated={(avatar) => {
          addToast('success', t('common.created'));
          setCreateOpen(false);
          setSelectedAvatarId(avatar.id);
          loadAvatars(false);
        }}
      />
      <CreateManagerAgentDialog
        open={createManagerOpen}
        onOpenChange={setCreateManagerOpen}
        teamId={teamId}
        onCreated={(agent) => {
          addToast('success', t('common.created'));
          setCreateManagerOpen(false);
          setAgents((prev) => [agent, ...prev.filter((item) => item.id !== agent.id)]);
          setBootstrapManagerAgentId(agent.id);
          setSelectedAvatarId(null);
        }}
      />
    </div>
  );
}

