import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  Bot,
  Check,
  CircleSlash,
  Clock3,
  ExternalLink,
  FileText,
  Loader2,
  Plus,
  RefreshCw,
  ShieldAlert,
  Sparkles,
  UserRound,
  Users,
} from 'lucide-react';
import { AgentTypeBadge, resolveAgentVisualType } from '../agent/AgentTypeBadge';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '../ui/dialog';
import {
  avatarPortalApi,
  type AvatarGovernanceEventPayload,
  type AvatarGovernanceQueueItemPayload,
  type AvatarInstanceProjection,
  type AvatarWorkbenchSnapshotPayload,
  type PortalDetail,
  type PortalDocumentAccessMode,
  type PortalSummary,
} from '../../api/avatarPortal';
import { chatApi, type ChatSessionEvent } from '../../api/chat';
import { BUILTIN_EXTENSIONS, agentApi, type TeamAgent } from '../../api/agent';
import {
  documentApi,
  type DocumentSummary,
  type LockInfo,
  type VersionSummary,
} from '../../api/documents';
import {
  missionApi,
  type MissionArtifact,
  type MissionDetail,
  type MissionListItem,
} from '../../api/mission';
import { ChatConversation, type ChatRuntimeEvent } from '../chat/ChatConversation';
import type { ChatInputQuickActionGroup } from '../chat/ChatInput';
import type { ChatInputComposeRequest } from '../chat/ChatInput';
import { DocumentEditor } from '../documents/DocumentEditor';
import { DocumentPicker } from '../documents/DocumentPicker';
import { VersionDiff } from '../documents/VersionDiff';
import { VersionTimeline } from '../documents/VersionTimeline';
import { useToast } from '../../contexts/ToastContext';
import { CreateAvatarDialog } from './digital-avatar/CreateAvatarDialog';
import { CreateManagerAgentDialog } from './digital-avatar/CreateManagerAgentDialog';
import { DigitalAvatarGuide } from './digital-avatar/DigitalAvatarGuide';
import { formatDateTime, formatRelativeTime } from '../../utils/format';
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

const DocumentPreview = lazy(() =>
  import('../documents/DocumentPreview').then((module) => ({ default: module.DocumentPreview })),
);

function DocumentPreviewLoading() {
  return (
    <div className="flex h-full min-h-[320px] items-center justify-center rounded-lg border border-dashed border-border/60 bg-muted/10 text-sm text-muted-foreground">
      正在加载文档预览...
    </div>
  );
}

interface DigitalAvatarSectionProps {
  teamId: string;
  canManage: boolean;
}

type AvatarFilter = 'all' | 'external' | 'internal';
type WorkspaceTab = 'workspace' | 'guide';
type InspectorTab = 'overview' | 'permissions' | 'governance' | 'logs' | 'publish';
type RuntimeLogFilter = 'pending' | 'all';
type GovernanceKindFilter = 'all' | 'capability' | 'proposal' | 'ticket' | 'runtime';
type GovernanceRiskFilter = 'all' | 'low' | 'medium' | 'high';
type PersistedEventFilter = 'all' | 'error' | 'tool' | 'thinking' | 'status';
type PublishViewMode = 'visitor' | 'preview' | 'test';
type WorkEntryMode = 'ask' | 'task' | 'collaborate' | 'mission' | null;
type WorkspaceDocumentMode = 'preview' | 'guide' | 'edit' | 'versions' | 'diff';
type WorkspaceObjectKind = 'document' | 'presentation' | 'page' | 'data' | 'other';
type WorkspaceRecommendation = {
  key: string;
  label: string;
  description: string;
  prompt: string;
  mode: WorkEntryMode;
  docId?: string;
};
type ManagerReportKind = 'delivery' | 'progress' | 'runtime' | 'governance';
type RuntimeSuggestion = RuntimeLogEntry;
type PersistedEventLoadMode = 'latest' | 'older' | 'incremental';
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

function renderCapabilityChipList(
  items: Array<{ id: string; name: string }>,
  emptyLabel: string,
) {
  if (items.length === 0) {
    return <span className="text-sm text-muted-foreground">{emptyLabel}</span>;
  }

  return (
    <div className="mt-1 flex flex-wrap gap-2">
      {items.map((item) => (
        <Badge key={`${item.id}-${item.name}`} variant="secondary" className="text-[11px]">
          {item.name}
        </Badge>
      ))}
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

function normalizeGovernanceRisk(value: string | null | undefined): GovernanceRiskFilter {
  const normalized = String(value || '').trim().toLowerCase();
  if (normalized === 'high' || normalized === 'medium' || normalized === 'low') {
    return normalized as GovernanceRiskFilter;
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

function SummaryPill({
  label,
  value,
  accent = false,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] ${
        accent
          ? 'border-primary/25 bg-primary/5 text-foreground'
          : 'border-border/60 bg-muted/25 text-muted-foreground'
      }`}
    >
      <span className="text-[10px] text-muted-foreground">{label}</span>
      <span className="font-medium text-foreground">{value}</span>
    </span>
  );
}

function workspaceSurfaceTitle(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t('digitalAvatar.workspace.surfacePresentationTitle', '演示材料工作面');
    case 'page':
      return t('digitalAvatar.workspace.surfacePageTitle', '页面协作面');
    case 'data':
      return t('digitalAvatar.workspace.surfaceDataTitle', '数据处理面');
    case 'document':
      return t('digitalAvatar.workspace.surfaceDocumentTitle', '文档协作面');
    default:
      return t('digitalAvatar.workspace.surfaceGenericTitle', '对象协作面');
  }
}

function workspaceSurfaceDescription(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t(
        'digitalAvatar.workspace.surfacePresentationHint',
        '围绕当前演示材料查看结构、页间叙事和版本变化，再决定是直接优化还是整理成长程任务。',
      );
    case 'page':
      return t(
        'digitalAvatar.workspace.surfacePageHint',
        '围绕当前页面查看内容、结构与边界说明，再决定是直接审阅、修改还是做发布前检查。',
      );
    case 'data':
      return t(
        'digitalAvatar.workspace.surfaceDataHint',
        '围绕当前数据对象查看内容、版本和差异，再决定是提炼结论、继续整理还是转成长程委托。',
      );
    case 'document':
      return t(
        'digitalAvatar.workspace.surfaceDocumentHint',
        '围绕当前文档查看内容、版本与差异，再决定是直接审阅、改写还是转成交付任务。',
      );
    default:
      return t(
        'digitalAvatar.workspace.surfaceGenericHint',
        '围绕当前对象查看内容和版本，再决定是共同处理、整理任务还是先让管理 Agent 给出建议。',
      );
  }
}

function defaultWorkspaceDocumentMode(
  kind: WorkspaceObjectKind | null | undefined,
): WorkspaceDocumentMode {
  return kind && kind !== 'document' ? 'guide' : 'preview';
}

function workspaceSurfaceGuideTitle(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t('digitalAvatar.workspace.surfaceGuidePresentationTitle', '先确定叙事和改版方式');
    case 'page':
      return t('digitalAvatar.workspace.surfaceGuidePageTitle', '先确认页面体验和发布边界');
    case 'data':
      return t('digitalAvatar.workspace.surfaceGuideDataTitle', '先明确结论、结构和交付形式');
    case 'document':
      return t('digitalAvatar.workspace.surfaceGuideDocumentTitle', '先审阅内容，再决定修改方式');
    default:
      return t('digitalAvatar.workspace.surfaceGuideGenericTitle', '先明确处理方式，再开始协作');
  }
}

function workspaceSurfaceFocusItems(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string[] {
  switch (kind) {
    case 'presentation':
      return [
        t('digitalAvatar.workspace.surfaceFocusPresentation1', '先看大纲、页间叙事和重点页面是否完整。'),
        t('digitalAvatar.workspace.surfaceFocusPresentation2', '优先决定是直接调结构，还是整理成改版任务。'),
        t('digitalAvatar.workspace.surfaceFocusPresentation3', '涉及补页、重写内容或补数据时，优先转成长程委托。'),
      ];
    case 'page':
      return [
        t('digitalAvatar.workspace.surfaceFocusPage1', '先确认访客第一屏看到什么，再看结构和边界说明。'),
        t('digitalAvatar.workspace.surfaceFocusPage2', '内容改动和发布检查最好分两步处理，避免一次性混在一起。'),
        t('digitalAvatar.workspace.surfaceFocusPage3', '涉及对外承诺、权限或品牌内容时，先生成发布前检查清单。'),
      ];
    case 'data':
      return [
        t('digitalAvatar.workspace.surfaceFocusData1', '先搞清字段、口径和数据质量，再提炼结论。'),
        t('digitalAvatar.workspace.surfaceFocusData2', '数据对象更适合先解释结构，再决定清洗、分析还是转任务。'),
        t('digitalAvatar.workspace.surfaceFocusData3', '涉及多表整理、分析产出或图表生成时，建议直接发起长程委托。'),
      ];
    case 'document':
      return [
        t('digitalAvatar.workspace.surfaceFocusDocument1', '先预览内容，再决定是解释、审阅、改写还是做差异对比。'),
        t('digitalAvatar.workspace.surfaceFocusDocument2', '需要修改时，优先进入编辑或版本视图，不要只在聊天里描述。'),
        t('digitalAvatar.workspace.surfaceFocusDocument3', '涉及正式内容发布或高风险改写时，先让管理 Agent 给出处理方式。'),
      ];
    default:
      return [
        t('digitalAvatar.workspace.surfaceFocusGeneric1', '先识别当前对象类型，再选择适合的协作方式。'),
        t('digitalAvatar.workspace.surfaceFocusGeneric2', '简单问题直接问，复杂改动转任务，持续处理进入长程委托。'),
        t('digitalAvatar.workspace.surfaceFocusGeneric3', '无法判断时，先让管理 Agent 解释边界和下一步。'),
      ];
  }
}

function workspaceSurfaceDeliverables(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string[] {
  switch (kind) {
    case 'presentation':
      return [
        t('digitalAvatar.workspace.surfaceDeliverablePresentation1', '改版结构建议'),
        t('digitalAvatar.workspace.surfaceDeliverablePresentation2', '页级修改任务'),
        t('digitalAvatar.workspace.surfaceDeliverablePresentation3', '新版演示稿'),
      ];
    case 'page':
      return [
        t('digitalAvatar.workspace.surfaceDeliverablePage1', '页面审阅结论'),
        t('digitalAvatar.workspace.surfaceDeliverablePage2', '发布前检查清单'),
        t('digitalAvatar.workspace.surfaceDeliverablePage3', '改版页面内容'),
      ];
    case 'data':
      return [
        t('digitalAvatar.workspace.surfaceDeliverableData1', '结构说明'),
        t('digitalAvatar.workspace.surfaceDeliverableData2', '重点结论'),
        t('digitalAvatar.workspace.surfaceDeliverableData3', '分析任务方案'),
      ];
    case 'document':
      return [
        t('digitalAvatar.workspace.surfaceDeliverableDocument1', '内容审阅意见'),
        t('digitalAvatar.workspace.surfaceDeliverableDocument2', '草稿修改版本'),
        t('digitalAvatar.workspace.surfaceDeliverableDocument3', '版本差异说明'),
      ];
    default:
      return [
        t('digitalAvatar.workspace.surfaceDeliverableGeneric1', '处理建议'),
        t('digitalAvatar.workspace.surfaceDeliverableGeneric2', '下一步动作'),
        t('digitalAvatar.workspace.surfaceDeliverableGeneric3', '可交付结果'),
      ];
  }
}

function workspaceSurfaceWorkflowTitle(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t('digitalAvatar.workspace.surfaceWorkflowPresentationTitle', '推荐推进路径');
    case 'page':
      return t('digitalAvatar.workspace.surfaceWorkflowPageTitle', '推荐推进路径');
    case 'data':
      return t('digitalAvatar.workspace.surfaceWorkflowDataTitle', '推荐推进路径');
    case 'document':
      return t('digitalAvatar.workspace.surfaceWorkflowDocumentTitle', '推荐推进路径');
    default:
      return t('digitalAvatar.workspace.surfaceWorkflowGenericTitle', '推荐推进路径');
  }
}

function workspaceSurfaceWorkflowSteps(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string[] {
  switch (kind) {
    case 'presentation':
      return [
        t('digitalAvatar.workspace.surfaceWorkflowPresentation1', '先确认这份演示材料要服务谁、解决什么问题。'),
        t('digitalAvatar.workspace.surfaceWorkflowPresentation2', '再看结构、大纲和重点页面，决定是微调还是整包改版。'),
        t('digitalAvatar.workspace.surfaceWorkflowPresentation3', '需要补页、补图、补数据时，优先转成长程委托。'),
      ];
    case 'page':
      return [
        t('digitalAvatar.workspace.surfaceWorkflowPage1', '先看访客首屏和页面结构，确认主叙事是否正确。'),
        t('digitalAvatar.workspace.surfaceWorkflowPage2', '再处理文案、边界和入口动作，必要时补发布前检查。'),
        t('digitalAvatar.workspace.surfaceWorkflowPage3', '涉及正式上线、对外承诺或品牌内容时，先走检查再改。'),
      ];
    case 'data':
      return [
        t('digitalAvatar.workspace.surfaceWorkflowData1', '先解释数据结构、字段口径和主要问题。'),
        t('digitalAvatar.workspace.surfaceWorkflowData2', '再决定是提炼结论、整理结果，还是继续做分析任务。'),
        t('digitalAvatar.workspace.surfaceWorkflowData3', '如果要跨表整理、图表生成或持续分析，优先转成长程委托。'),
      ];
    case 'document':
      return [
        t('digitalAvatar.workspace.surfaceWorkflowDocument1', '先预览原文，明确是解释、审阅还是改写。'),
        t('digitalAvatar.workspace.surfaceWorkflowDocument2', '涉及修改时优先进入编辑、版本或 Diff，而不是只在对话里描述。'),
        t('digitalAvatar.workspace.surfaceWorkflowDocument3', '高风险或正式内容改写先交给管理 Agent 判断处理方式。'),
      ];
    default:
      return [
        t('digitalAvatar.workspace.surfaceWorkflowGeneric1', '先识别对象类型和当前目标。'),
        t('digitalAvatar.workspace.surfaceWorkflowGeneric2', '再决定是直接协作、交任务，还是进入长程委托。'),
        t('digitalAvatar.workspace.surfaceWorkflowGeneric3', '不确定时，先让管理 Agent 给出下一步建议。'),
      ];
  }
}

function workspaceSurfaceBoundaryItems(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string[] {
  const shared = [
    t('digitalAvatar.workspace.surfaceBoundaryShared1', '仍然受绑定对象范围、文档模式和当前允许能力约束。'),
    t('digitalAvatar.workspace.surfaceBoundaryShared2', '不确定能否直接处理时，先生成建议或转给管理 Agent 判断。'),
  ];
  switch (kind) {
    case 'presentation':
      return [
        t('digitalAvatar.workspace.surfaceBoundaryPresentation1', '不要默认重写整份演示材料，先锁定需要调整的页面或结构。'),
        ...shared,
      ];
    case 'page':
      return [
        t('digitalAvatar.workspace.surfaceBoundaryPage1', '页面对外可见内容、边界说明和发布动作要分开确认。'),
        ...shared,
      ];
    case 'data':
      return [
        t('digitalAvatar.workspace.surfaceBoundaryData1', '不要直接假设字段含义或统计口径，先解释再下结论。'),
        ...shared,
      ];
    case 'document':
      return [
        t('digitalAvatar.workspace.surfaceBoundaryDocument1', '编辑前先确认是否需要审阅、Diff 或协作草稿，而不是直接覆盖内容。'),
        ...shared,
      ];
    default:
      return shared;
  }
}

function workspacePreviewModeLabel(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t('digitalAvatar.workspace.docModePresentationPreview', '演示视图');
    case 'page':
      return t('digitalAvatar.workspace.docModePagePreview', '页面视图');
    case 'data':
      return t('digitalAvatar.workspace.docModeDataPreview', '数据视图');
    case 'document':
      return t('digitalAvatar.workspace.docModePreview', '预览');
    default:
      return t('digitalAvatar.workspace.docModeObjectPreview', '对象视图');
  }
}

function workspaceHistoryModeLabel(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t('digitalAvatar.workspace.docModePresentationHistory', '版本 / 改版');
    case 'page':
      return t('digitalAvatar.workspace.docModePageHistory', '变更 / 检查');
    case 'data':
      return t('digitalAvatar.workspace.docModeDataHistory', '历史 / 差异');
    case 'document':
      return t('digitalAvatar.workspace.docModeVersions', '版本 / Diff');
    default:
      return t('digitalAvatar.workspace.docModeObjectHistory', '历史 / 变化');
  }
}

function workspacePreviewSurfaceTitle(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t('digitalAvatar.workspace.previewPresentationTitle', '演示材料主面');
    case 'page':
      return t('digitalAvatar.workspace.previewPageTitle', '页面主面');
    case 'data':
      return t('digitalAvatar.workspace.previewDataTitle', '数据主面');
    case 'document':
      return t('digitalAvatar.workspace.previewDocumentTitle', '文档主面');
    default:
      return t('digitalAvatar.workspace.previewGenericTitle', '对象主面');
  }
}

function workspacePreviewSurfaceHint(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'presentation':
      return t(
        'digitalAvatar.workspace.previewPresentationHint',
        '先看页面结构和叙事，再决定是小范围优化、页级改版，还是直接整理成长程委托。',
      );
    case 'page':
      return t(
        'digitalAvatar.workspace.previewPageHint',
        '先看访客视角和页面结构，再做文案调整、边界校验或发布前检查。',
      );
    case 'data':
      return t(
        'digitalAvatar.workspace.previewDataHint',
        '先看数据结构和主要内容，再决定是提炼结论、整理任务，还是继续进入分析流程。',
      );
    case 'document':
      return t(
        'digitalAvatar.workspace.previewDocumentHint',
        '直接围绕内容预览、编辑、版本和差异继续协作处理。',
      );
    default:
      return t(
        'digitalAvatar.workspace.previewGenericHint',
        '先看对象内容，再决定是继续协作、补任务，还是转成长程委托。',
      );
  }
}

function workspacePreviewSurfaceHighlights(
  kind: WorkspaceObjectKind | null | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string[] {
  switch (kind) {
    case 'presentation':
      return [
        t('digitalAvatar.workspace.previewPresentationHighlight1', '优先关注叙事完整性、页间过渡和重点页面。'),
        t('digitalAvatar.workspace.previewPresentationHighlight2', '需要大改结构或补充内容时，优先转成长程委托。'),
      ];
    case 'page':
      return [
        t('digitalAvatar.workspace.previewPageHighlight1', '优先关注首屏表达、信息层级和对外边界说明。'),
        t('digitalAvatar.workspace.previewPageHighlight2', '涉及上线发布时，先补发布前检查而不是直接变更。'),
      ];
    case 'data':
      return [
        t('digitalAvatar.workspace.previewDataHighlight1', '优先确认字段、结构和口径，再生成结论。'),
        t('digitalAvatar.workspace.previewDataHighlight2', '涉及多表分析或复杂清洗时，优先交给长程委托。'),
      ];
    case 'document':
      return [
        t('digitalAvatar.workspace.previewDocumentHighlight1', '优先在预览、编辑和版本之间切换，而不是只靠对话描述。'),
        t('digitalAvatar.workspace.previewDocumentHighlight2', '涉及正式内容改写时，先确认审阅与 Diff 路径。'),
      ];
    default:
      return [
        t('digitalAvatar.workspace.previewGenericHighlight1', '优先确认当前对象适合哪种处理方式。'),
        t('digitalAvatar.workspace.previewGenericHighlight2', '复杂处理先拆成任务，再交给管理 Agent 编排。'),
      ];
  }
}

function detectWorkspaceObjectKind(doc: DocumentSummary): WorkspaceObjectKind {
  const mime = (doc.mime_type || '').toLowerCase();
  const name = `${doc.display_name || ''} ${doc.name || ''}`.toLowerCase();

  if (
    mime.includes('presentation')
    || mime.includes('powerpoint')
    || name.endsWith('.ppt')
    || name.endsWith('.pptx')
    || name.endsWith('.key')
  ) {
    return 'presentation';
  }
  if (
    mime.includes('html')
    || mime.includes('svg')
    || name.endsWith('.html')
    || name.endsWith('.htm')
    || name.endsWith('.xml')
  ) {
    return 'page';
  }
  if (
    mime.includes('csv')
    || mime.includes('spreadsheet')
    || mime.includes('excel')
    || mime.includes('json')
    || mime.includes('yaml')
    || mime.includes('toml')
    || name.endsWith('.csv')
    || name.endsWith('.xlsx')
    || name.endsWith('.xls')
    || name.endsWith('.json')
    || name.endsWith('.yaml')
    || name.endsWith('.yml')
    || name.endsWith('.toml')
  ) {
    return 'data';
  }
  if (
    mime.includes('pdf')
    || mime.includes('markdown')
    || mime.includes('text')
    || mime.includes('word')
    || mime.includes('document')
    || name.endsWith('.pdf')
    || name.endsWith('.md')
    || name.endsWith('.txt')
    || name.endsWith('.doc')
    || name.endsWith('.docx')
  ) {
    return 'document';
  }
  return 'other';
}

function workspaceObjectKindLabel(
  kind: WorkspaceObjectKind,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  switch (kind) {
    case 'document':
      return t('digitalAvatar.workspace.objectKindDocument', '文档');
    case 'presentation':
      return t('digitalAvatar.workspace.objectKindPresentation', '演示材料');
    case 'page':
      return t('digitalAvatar.workspace.objectKindPage', '页面');
    case 'data':
      return t('digitalAvatar.workspace.objectKindData', '数据');
    default:
      return t('digitalAvatar.workspace.objectKindOther', '对象');
  }
}

function workspaceObjectKindBadgeClass(kind: WorkspaceObjectKind): string {
  switch (kind) {
    case 'document':
      return 'border-sky-200 bg-sky-50 text-sky-700';
    case 'presentation':
      return 'border-fuchsia-200 bg-fuchsia-50 text-fuchsia-700';
    case 'page':
      return 'border-emerald-200 bg-emerald-50 text-emerald-700';
    case 'data':
      return 'border-amber-200 bg-amber-50 text-amber-700';
    default:
      return 'border-border/60 bg-muted/30 text-muted-foreground';
  }
}

function missionStatusBadgeClass(status: string): string {
  switch (status) {
    case 'running':
      return 'border-status-success/35 bg-status-success/10 text-status-success-text';
    case 'completed':
      return 'border-status-success/35 bg-status-success/10 text-status-success-text';
    case 'failed':
    case 'cancelled':
      return 'border-status-error/35 bg-status-error/10 text-status-error-text';
    case 'paused':
      return 'border-status-warning/35 bg-status-warning/10 text-status-warning-text';
    default:
      return 'border-border/60 bg-muted/30 text-muted-foreground';
  }
}

function avatarStatusBadgeClass(status: string): string {
  switch (status) {
    case 'published':
      return 'border-status-success/35 bg-status-success/10 text-status-success-text';
    case 'draft':
      return 'border-status-warning/35 bg-status-warning/10 text-status-warning-text';
    case 'disabled':
    case 'archived':
      return 'border-border/60 bg-muted/30 text-muted-foreground';
    default:
      return 'border-border/60 bg-background text-muted-foreground';
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

function avatarStatusDotClass(status: string): string {
  switch (status) {
    case 'published':
      return 'bg-status-success';
    case 'draft':
      return 'bg-status-warning';
    case 'disabled':
    case 'archived':
      return 'bg-muted-foreground/60';
    default:
      return 'bg-border';
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

function eventSummary(event: ChatSessionEvent): string {
  const payload = event.payload || {};
  if (event.event_type === 'text' || event.event_type === 'thinking') {
    return String(payload.content || '').trim();
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

  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [publishingAvatar, setPublishingAvatar] = useState(false);
  const [updatingPublicConfig, setUpdatingPublicConfig] = useState(false);
  const [tab, setTab] = useState<WorkspaceTab>('workspace');
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>('overview');
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const [focusMode, setFocusMode] = useState(false);
  const [workspacePanelOpen, setWorkspacePanelOpen] = useState(false);
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
  const [workbenchSnapshot, setWorkbenchSnapshot] = useState<AvatarWorkbenchSnapshotPayload | null>(null);
  const governanceRef = useRef(governance);
  const selectedAvatarRef = useRef<PortalDetail | null>(selectedAvatar);
  const governancePersistQueueRef = useRef<Promise<void>>(Promise.resolve());
  const governancePersistInFlightRef = useRef(0);
  const [savingGovernance, setSavingGovernance] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [createManagerOpen, setCreateManagerOpen] = useState(false);
  const [managerSessionId, setManagerSessionId] = useState<string | null>(null);
  const [managerProcessing, setManagerProcessing] = useState(false);
  const [runtimeLogFilter, setRuntimeLogFilter] = useState<RuntimeLogFilter>('pending');
  const [governanceKindFilter, setGovernanceKindFilter] = useState<GovernanceKindFilter>('all');
  const [governanceRiskFilter, setGovernanceRiskFilter] = useState<GovernanceRiskFilter>('all');
  const [governanceSearch, setGovernanceSearch] = useState('');
  const [governanceManualOnly, setGovernanceManualOnly] = useState(false);
  const [bootstrapManagerAgentId, setBootstrapManagerAgentId] = useState('');
  const [managerComposeRequest, setManagerComposeRequest] = useState<ChatInputComposeRequest | null>(null);
  const [workEntryMode, setWorkEntryMode] = useState<WorkEntryMode>('ask');
  const [taskGoalDraft, setTaskGoalDraft] = useState('');
  const [taskOutcomeDraft, setTaskOutcomeDraft] = useState('');
  const [missionGoalDraft, setMissionGoalDraft] = useState('');
  const [missionContextDraft, setMissionContextDraft] = useState('');
  const [creatingMission, setCreatingMission] = useState(false);
  const [workspaceMissions, setWorkspaceMissions] = useState<MissionListItem[]>([]);
  const [workspaceMissionsLoading, setWorkspaceMissionsLoading] = useState(false);
  const [workspaceDocuments, setWorkspaceDocuments] = useState<DocumentSummary[]>([]);
  const [workspaceDocumentsLoading, setWorkspaceDocumentsLoading] = useState(false);
  const [permissionSelectorDialog, setPermissionSelectorDialog] = useState<'extensions' | 'skills' | null>(null);
  const [showPermissionDocPicker, setShowPermissionDocPicker] = useState(false);
  const [permissionSelectedDocIds, setPermissionSelectedDocIds] = useState<string[]>([]);
  const [permissionSelectedExtensions, setPermissionSelectedExtensions] = useState<string[]>([]);
  const [permissionSelectedSkillIds, setPermissionSelectedSkillIds] = useState<string[]>([]);
  const [permissionDocumentAccessMode, setPermissionDocumentAccessMode] = useState<PortalDocumentAccessMode>('read_only');
  const [savingPermissionConfig, setSavingPermissionConfig] = useState(false);
  const [selectedWorkspaceDocumentId, setSelectedWorkspaceDocumentId] = useState<string | null>(null);
  const [workspaceDocumentMode, setWorkspaceDocumentMode] = useState<WorkspaceDocumentMode>('preview');
  const [workspaceDocumentText, setWorkspaceDocumentText] = useState('');
  const [workspaceDocumentLoading, setWorkspaceDocumentLoading] = useState(false);
  const [workspaceDocumentLock, setWorkspaceDocumentLock] = useState<LockInfo | null>(null);
  const [workspaceDocumentCompareVersions, setWorkspaceDocumentCompareVersions] = useState<
    [VersionSummary, VersionSummary] | null
  >(null);
  const [workspaceMissionDetails, setWorkspaceMissionDetails] = useState<Record<string, MissionDetail>>({});
  const [workspaceMissionArtifacts, setWorkspaceMissionArtifacts] = useState<Record<string, MissionArtifact[]>>({});
  const [workspaceMissionMetaLoading, setWorkspaceMissionMetaLoading] = useState(false);
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

  const sessionStoragePrefix = `digital_avatar_manager_session:v1:${teamId}:`;
  const pendingManagerComposeStoragePrefix = `${MANAGER_COMPOSE_STORAGE_PREFIX}${teamId}:`;
  const pendingManagerFocusStorageKey = `${MANAGER_FOCUS_STORAGE_PREFIX}${teamId}`;

  const managerAgentId = selectedAvatar?.codingAgentId || selectedAvatar?.agentId || null;
  const managerGroupOptions = useMemo(
    () => resolveManagerGroupCandidates(agents, avatars),
    [agents, avatars],
  );

  const fallbackManagerAgentId = managerGroupOptions[0]?.id || null;
  const selectedManagerGroupId = bootstrapManagerAgentId || fallbackManagerAgentId;
  const managerScopedAvatars = useMemo(() => {
    const scopeManagerId = selectedAvatar
      ? (selectedAvatar.codingAgentId || selectedAvatar.agentId || null)
      : selectedManagerGroupId;
    if (!scopeManagerId) return avatars;
    return avatars.filter((avatar) => getDigitalAvatarManagerId(avatar) === scopeManagerId);
  }, [avatars, selectedAvatar, selectedManagerGroupId]);

  const visibleAvatars = useMemo(() => {
    const base = managerScopedAvatars;
    if (filter === 'all') return base;
    return base.filter((avatar) => detectAvatarType(avatar, avatarProjectionMap[avatar.id]) === filter);
  }, [avatarProjectionMap, filter, managerScopedAvatars]);
  const managerGroupStats = useMemo(() => {
    const total = managerScopedAvatars.length;
    const external = managerScopedAvatars.filter((avatar) => detectAvatarType(avatar, avatarProjectionMap[avatar.id]) === 'external').length;
    const internal = managerScopedAvatars.filter((avatar) => detectAvatarType(avatar, avatarProjectionMap[avatar.id]) === 'internal').length;
    const published = managerScopedAvatars.filter((avatar) => normalizeAvatarStatus(avatar) === 'published').length;
    const draft = managerScopedAvatars.filter((avatar) => ['draft', 'disabled', 'archived'].includes(normalizeAvatarStatus(avatar))).length;
    const pending = managerScopedAvatars.reduce(
      (sum, avatar) => sum + getAvatarProjectionPendingCount(avatarProjectionMap[avatar.id]),
      0,
    );
    return { total, external, internal, published, draft, pending };
  }, [avatarProjectionMap, managerScopedAvatars]);
  const managerPreviewAvatars = useMemo(
    () => managerScopedAvatars.slice(0, 3),
    [managerScopedAvatars],
  );
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
  const effectiveManagerAgentId = selectedAvatar
    ? managerAgentId
    : selectedManagerGroupId;
  const managerAgent = useMemo(
    () => agents.find(agent => agent.id === effectiveManagerAgentId) || null,
    [agents, effectiveManagerAgentId]
  );
  const selectedAvatarServiceAgentId = selectedAvatar?.serviceAgentId || selectedAvatar?.agentId || null;
  const selectedAvatarServiceAgent = useMemo(
    () => agents.find((agent) => agent.id === selectedAvatarServiceAgentId) || null,
    [agents, selectedAvatarServiceAgentId],
  );

  const selectedAvatarType = selectedAvatar ? detectAvatarType(selectedAvatar, avatarProjectionMap[selectedAvatar.id]) : 'unknown';
  const selectedAvatarStatus = useMemo(
    () => normalizeAvatarStatus(selectedAvatar),
    [selectedAvatar],
  );
  const selectedAvatarEffectivePublicConfig = selectedAvatar?.effectivePublicConfig || null;
  const selectedAvatarDocumentAccessMode =
    selectedAvatarEffectivePublicConfig?.effectiveDocumentAccessMode || selectedAvatar?.documentAccessMode;
  const selectedAvatarShowChatWidget = useMemo(() => {
    const raw = (selectedAvatar?.settings as Record<string, unknown> | undefined)?.showChatWidget;
    return typeof raw === 'boolean' ? raw : true;
  }, [selectedAvatar?.settings]);
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
    if (effectiveIds && effectiveIds.length > 0) {
      return effectiveIds.map((id) => ({
        id,
        name: resolveExtensionLabel(id),
      }));
    }
    return selectedAvatarEnabledExtensionEntries;
  }, [selectedAvatarEffectivePublicConfig?.effectiveAllowedExtensions, selectedAvatarEnabledExtensionEntries]);
  const selectedAvatarEffectiveSkillEntries = useMemo(() => {
    const effectiveIds = selectedAvatarEffectivePublicConfig?.effectiveAllowedSkillIds;
    if (effectiveIds && effectiveIds.length > 0) {
      return effectiveIds.map((id) => {
        const match = selectedAvatarAssignedSkillEntries.find((item) => item.id === id);
        return {
          id,
          name: match?.name || id,
        };
      });
    }
    return selectedAvatarAssignedSkillEntries;
  }, [selectedAvatarAssignedSkillEntries, selectedAvatarEffectivePublicConfig?.effectiveAllowedSkillIds]);
  const selectedAvatarRuntimeExtensionOptions = useMemo(
    () => getRuntimeExtensionOptions(selectedAvatarServiceAgent),
    [selectedAvatarServiceAgent],
  );
  const permissionSelectedDocuments = useMemo(
    () => workspaceDocuments.filter((doc) => permissionSelectedDocIds.includes(doc.id)),
    [permissionSelectedDocIds, workspaceDocuments],
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
        (selectedAvatar?.allowedExtensions && selectedAvatar.allowedExtensions.length > 0)
          || (selectedAvatar?.allowedSkillIds && selectedAvatar.allowedSkillIds.length > 0),
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
    () => formatPortalOutputForm(selectedAvatar?.outputForm, t),
    [selectedAvatar?.outputForm, t],
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
  const publishPath = selectedAvatar?.slug ? `/p/${selectedAvatar.slug}` : '';
  const selectedAvatarPublicUrl = selectedAvatarEffectivePublicConfig?.publicAccessEnabled
    ? (selectedAvatar?.publicUrl || publishPath || '')
    : '';
  const selectedAvatarPreviewUrl = selectedAvatar?.previewUrl || '';
  const selectedAvatarTestUrl = selectedAvatarEffectivePublicConfig?.publicAccessEnabled
    ? (selectedAvatar?.testPublicUrl || '')
    : '';
  const selectedAvatarBoundDocCount = selectedAvatar?.boundDocumentIds?.length || 0;
  const selectedWorkspaceDocument = useMemo(
    () => workspaceDocuments.find((doc) => doc.id === selectedWorkspaceDocumentId) || null,
    [selectedWorkspaceDocumentId, workspaceDocuments],
  );
  const workspaceObjectSummaries = useMemo(
    () =>
      workspaceDocuments.map((doc) => {
        const kind = detectWorkspaceObjectKind(doc);
        return {
          doc,
          kind,
          kindLabel: workspaceObjectKindLabel(kind, t),
        };
      }),
    [t, workspaceDocuments],
  );
  const selectedWorkspaceObjectSummary = useMemo(
    () =>
      workspaceObjectSummaries.find((item) => item.doc.id === selectedWorkspaceDocumentId)
      || null,
    [selectedWorkspaceDocumentId, workspaceObjectSummaries],
  );
  const workspaceObjectKindCounts = useMemo(() => {
    const counts: Record<WorkspaceObjectKind, number> = {
      document: 0,
      presentation: 0,
      page: 0,
      data: 0,
      other: 0,
    };
    workspaceObjectSummaries.forEach((item) => {
      counts[item.kind] += 1;
    });
    return counts;
  }, [workspaceObjectSummaries]);
  const workspaceObjectSummaryPills = useMemo(
    () =>
      (Object.entries(workspaceObjectKindCounts) as Array<[WorkspaceObjectKind, number]>)
        .filter(([, count]) => count > 0)
        .map(([kind, count]) => ({
          key: kind,
          label: workspaceObjectKindLabel(kind, t),
          count,
        })),
    [t, workspaceObjectKindCounts],
  );
  const preferredWorkspaceObjectSummary = selectedWorkspaceObjectSummary || workspaceObjectSummaries[0] || null;
  const workspaceRecommendations = useMemo<WorkspaceRecommendation[]>(() => {
    if (!selectedAvatar) return [];

    const objectSummary = preferredWorkspaceObjectSummary;
    if (!objectSummary) {
      return [
        {
          key: 'clarify',
          label: t('digitalAvatar.workspace.recommendationClarify', { defaultValue: '先澄清需求' }),
          description: t(
            'digitalAvatar.workspace.recommendationClarifyDesc',
            { defaultValue: '让管理 Agent 先判断该直接答复、补对象，还是进入长程委托。' },
          ),
          prompt: t(
            'digitalAvatar.workspace.recommendationClarifyPrompt',
            {
              defaultValue:
                '请先根据当前岗位和我的目标，判断适合直接答复、共同处理还是长程委托，并给出下一步建议。',
            },
          ),
          mode: 'ask',
        },
      ];
    }

    const { doc, kind, kindLabel } = objectSummary;
    const objectName = doc.display_name || doc.name;
    const baseContext = t(
      'digitalAvatar.workspace.recommendationBaseContext',
      {
        defaultValue: '请围绕当前工作对象「{{name}}」（{{kind}}）处理下面的需求，并先说明边界、方式和下一步。',
        name: objectName,
        kind: kindLabel,
      },
    );

    if (kind === 'presentation') {
      return [
        {
          key: 'presentation-structure',
          label: t('digitalAvatar.workspace.recommendationPresentationStructure', { defaultValue: '优化结构' }),
          description: t(
            'digitalAvatar.workspace.recommendationPresentationStructureDesc',
            { defaultValue: '先检查大纲、页间逻辑和讲述顺序，再给出重组建议。' },
          ),
          prompt: `${baseContext}\n${t(
            'digitalAvatar.workspace.recommendationPresentationStructurePrompt',
            { defaultValue: '请先审阅结构和叙事顺序，再提出优化方案，必要时建议进入长程委托。' },
          )}`,
          mode: 'collaborate',
          docId: doc.id,
        },
        {
          key: 'presentation-task',
          label: t('digitalAvatar.workspace.recommendationPresentationTask', { defaultValue: '生成改版任务' }),
          description: t(
            'digitalAvatar.workspace.recommendationPresentationTaskDesc',
            { defaultValue: '把补页、改稿、数据补充等复杂事项整理成可交付任务。' },
          ),
          prompt: `${baseContext}\n${t(
            'digitalAvatar.workspace.recommendationPresentationTaskPrompt',
            { defaultValue: '请基于当前演示材料生成一份改版任务方案，说明是否需要转成长程委托。' },
          )}`,
          mode: 'task',
          docId: doc.id,
        },
      ];
    }

    if (kind === 'page') {
      return [
        {
          key: 'page-review',
          label: t('digitalAvatar.workspace.recommendationPageReview', { defaultValue: '审阅页面' }),
          description: t(
            'digitalAvatar.workspace.recommendationPageReviewDesc',
            { defaultValue: '检查页面文案、结构和边界说明，再决定是直接修改还是提任务。' },
          ),
          prompt: `${baseContext}\n${t(
            'digitalAvatar.workspace.recommendationPageReviewPrompt',
            { defaultValue: '请先审阅这个页面的内容和结构，再给出优化建议与处理方式。' },
          )}`,
          mode: 'collaborate',
          docId: doc.id,
        },
        {
          key: 'page-publish',
          label: t('digitalAvatar.workspace.recommendationPagePublish', { defaultValue: '发布前检查' }),
          description: t(
            'digitalAvatar.workspace.recommendationPagePublishDesc',
            { defaultValue: '围绕可见内容、边界说明和访客体验生成上线前检查清单。' },
          ),
          prompt: `${baseContext}\n${t(
            'digitalAvatar.workspace.recommendationPagePublishPrompt',
            { defaultValue: '请生成这份页面的发布前检查清单，重点覆盖权限、说明和访客体验。' },
          )}`,
          mode: 'task',
          docId: doc.id,
        },
      ];
    }

    if (kind === 'data') {
      return [
        {
          key: 'data-summary',
          label: t('digitalAvatar.workspace.recommendationDataSummary', { defaultValue: '提炼结论' }),
          description: t(
            'digitalAvatar.workspace.recommendationDataSummaryDesc',
            { defaultValue: '先看字段和数据质量，再整理重点结论和说明。' },
          ),
          prompt: `${baseContext}\n${t(
            'digitalAvatar.workspace.recommendationDataSummaryPrompt',
            { defaultValue: '请先解释这份数据对象的字段和结构，再整理关键结论与风险点。' },
          )}`,
          mode: 'collaborate',
          docId: doc.id,
        },
        {
          key: 'data-task',
          label: t('digitalAvatar.workspace.recommendationDataTask', { defaultValue: '整理成任务' }),
          description: t(
            'digitalAvatar.workspace.recommendationDataTaskDesc',
            { defaultValue: '把清洗、分析、汇总等复杂处理整理为长程任务。' },
          ),
          prompt: `${baseContext}\n${t(
            'digitalAvatar.workspace.recommendationDataTaskPrompt',
            { defaultValue: '请把这份数据对象的整理、分析和交付工作拆成可执行任务，并判断是否需要长程委托。' },
          )}`,
          mode: 'task',
          docId: doc.id,
        },
      ];
    }

    return [
      {
        key: 'object-review',
        label: t('digitalAvatar.workspace.recommendationObjectReview', { defaultValue: '审阅对象' }),
        description: t(
          'digitalAvatar.workspace.recommendationObjectReviewDesc',
          { defaultValue: '先预览并理解当前对象，再给出修改、答复或协作建议。' },
        ),
        prompt: `${baseContext}\n${t(
          'digitalAvatar.workspace.recommendationObjectReviewPrompt',
          { defaultValue: '请先审阅当前对象，再说明适合直接答复、共同处理还是转成长程委托。' },
        )}`,
        mode: 'collaborate',
        docId: doc.id,
      },
      {
        key: 'object-task',
        label: t('digitalAvatar.workspace.recommendationObjectTask', { defaultValue: '交付任务' }),
        description: t(
          'digitalAvatar.workspace.recommendationObjectTaskDesc',
          { defaultValue: '把围绕当前对象的复杂改写、生产或交付工作整理成任务。' },
        ),
        prompt: `${baseContext}\n${t(
          'digitalAvatar.workspace.recommendationObjectTaskPrompt',
          { defaultValue: '请围绕当前对象整理一份交付任务方案，并说明是否要进入长程委托。' },
        )}`,
        mode: 'task',
        docId: doc.id,
      },
    ];
  }, [preferredWorkspaceObjectSummary, selectedAvatar, t]);
  const workspaceDocumentEditable = useMemo(() => {
    if (!selectedWorkspaceDocument) return false;
    if (selectedAvatarDocumentAccessMode === 'read_only') return false;
    const mime = (selectedWorkspaceDocument.mime_type || '').toLowerCase();
    return (
      mime.startsWith('text/')
      || mime.includes('json')
      || mime.includes('xml')
      || mime.includes('javascript')
      || mime.includes('typescript')
      || mime.includes('markdown')
      || mime.includes('yaml')
      || mime.includes('toml')
      || mime.includes('csv')
      || mime.includes('html')
    );
  }, [selectedAvatarDocumentAccessMode, selectedWorkspaceDocument]);
  const featuredWorkspaceMission = useMemo(
    () => workspaceMissions[0] || null,
    [workspaceMissions],
  );
  const featuredWorkspaceMissionDetail = featuredWorkspaceMission
    ? workspaceMissionDetails[featuredWorkspaceMission.mission_id]
    : undefined;
  const featuredWorkspaceMissionArtifacts = featuredWorkspaceMission
    ? (workspaceMissionArtifacts[featuredWorkspaceMission.mission_id] || [])
    : [];
  const latestCompletedWorkspaceMission = useMemo(
    () => workspaceMissions.find((mission) => mission.status === 'completed') || null,
    [workspaceMissions],
  );
  const latestCompletedWorkspaceMissionDetail = latestCompletedWorkspaceMission
    ? workspaceMissionDetails[latestCompletedWorkspaceMission.mission_id]
    : undefined;
  const latestAttentionWorkspaceMission = useMemo(
    () =>
      workspaceMissions.find((mission) => mission.status === 'failed' || mission.status === 'paused')
      || workspaceMissions.find((mission) => mission.status === 'running')
      || null,
    [workspaceMissions],
  );
  const permissionPreview = useMemo(
    () => buildPermissionPreview(selectedAvatarDocumentAccessMode, t),
    [selectedAvatarDocumentAccessMode, t],
  );
  const selectedAvatarLastActivityLabel = useMemo(() => {
    if (workbenchSnapshot?.summary.last_activity_at) {
      return formatRelativeTime(workbenchSnapshot.summary.last_activity_at);
    }
    if (!selectedAvatar?.updatedAt) return t('digitalAvatar.labels.unset');
    return formatRelativeTime(selectedAvatar.updatedAt);
  }, [selectedAvatar?.updatedAt, t, workbenchSnapshot?.summary.last_activity_at]);
  const selectedAvatarSummaryText = useMemo(() => {
    if (!selectedAvatar) {
      return t(
        'digitalAvatar.workspace.emptySummary',
        '尚未选择分身。先从左侧选择一个分身，或新建一个分身后再进入治理与协作。'
      );
    }
    const description = (selectedAvatar.description || '').trim();
    if (description) return description;
    return selectedAvatarType === 'internal'
      ? t(
          'digitalAvatar.workspace.internalSummaryFallback',
          '面向内部成员，适合流程执行、任务协同、知识检索和受控文档处理。'
        )
      : selectedAvatarType === 'external'
      ? t(
          'digitalAvatar.workspace.externalSummaryFallback',
          '面向客户、合作伙伴或外部访客，提供边界清晰的对外问答、协作与交付入口。'
        )
      : t(
          'digitalAvatar.workspace.genericSummaryFallback',
          '通过管理 Agent 统一创建、治理和优化当前分身。'
        );
  }, [selectedAvatar, selectedAvatarType, t]);
  const managerScopeSummaryText = useMemo(() => {
    const managerName = getAgentName(agents, effectiveManagerAgentId, t('digitalAvatar.states.noManagerAgent'));
    if (managerGroupStats.total === 0) {
      return t('digitalAvatar.workspace.managerScopeEmpty', {
        defaultValue: '{{manager}} 当前还没有分身。先创建一个分身，再通过管理对话补齐能力、权限和发布配置。',
        manager: managerName,
      });
    }
    return t('digitalAvatar.workspace.managerScopeSummary', {
      defaultValue:
        '{{manager}} 当前管理 {{total}} 个分身，其中对外 {{external}} 个、对内 {{internal}} 个、已发布 {{published}} 个，待处理治理事项 {{pending}} 项。',
      manager: managerName,
      total: managerGroupStats.total,
      external: managerGroupStats.external,
      internal: managerGroupStats.internal,
      published: managerGroupStats.published,
      pending: managerGroupStats.pending,
    });
  }, [agents, effectiveManagerAgentId, managerGroupStats, t]);
  const showWorkspaceBootstrapOnly = !selectedAvatar && managerGroupStats.total === 0;
  const roleOverviewPrimaryAction = useMemo(() => {
    if (selectedAvatar) {
      if (latestAttentionWorkspaceMission) {
        return latestAttentionWorkspaceMission.status === 'running'
          ? {
              kind: 'open_mission' as const,
              label: t('digitalAvatar.workspace.openMissionDetail', '查看进度'),
              missionId: latestAttentionWorkspaceMission.mission_id,
            }
          : {
              kind: 'resume_mission' as const,
              label: t('digitalAvatar.workspace.resumeMission', '继续处理'),
              missionId: latestAttentionWorkspaceMission.mission_id,
            };
      }
      if (workspaceObjectSummaries.length > 0) {
        return {
          kind: 'collaborate' as const,
          label: t('digitalAvatar.workspace.entryCollaborate', '共同处理'),
          docId: workspaceObjectSummaries[0]?.doc.id || null,
        };
      }
      return {
        kind: 'task' as const,
        label: t('digitalAvatar.workspace.entryTask', '交任务'),
      };
    }

    if (managerPreviewAvatars.length > 0) {
      return {
        kind: 'select_avatar' as const,
        label: t('digitalAvatar.workspace.selectFirstRole', {
          defaultValue: '进入最近岗位',
        }),
        avatarId: managerPreviewAvatars[0].id,
      };
    }

    return {
      kind: 'create_avatar' as const,
      label: t('digitalAvatar.actions.planRole', '发起岗位规划'),
    };
  }, [
    latestAttentionWorkspaceMission,
    managerPreviewAvatars,
    selectedAvatar,
    t,
    workspaceObjectSummaries,
  ]);
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
  const secondaryPublishLinks = useMemo(() => {
    const links: Array<{ key: PublishViewMode; label: string; url: string }> = [];
    if (selectedAvatarPreviewUrl && publishViewMode !== 'preview' && selectedAvatarPreviewUrl !== activePublishUrl) {
      links.push({
        key: 'preview',
        label: t('digitalAvatar.workspace.previewUrl', '管理预览'),
        url: selectedAvatarPreviewUrl,
      });
    }
    if (selectedAvatarTestUrl && publishViewMode !== 'test' && selectedAvatarTestUrl !== activePublishUrl) {
      links.push({
        key: 'test',
        label: t('digitalAvatar.workspace.testUrl', '测试入口'),
        url: selectedAvatarTestUrl,
      });
    }
    return links;
  }, [activePublishUrl, publishViewMode, selectedAvatarPreviewUrl, selectedAvatarTestUrl, t]);

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

  useEffect(() => {
    let cancelled = false;
    if (!effectiveManagerAgentId) {
      setWorkspaceMissions([]);
      return;
    }

    setWorkspaceMissionsLoading(true);
    missionApi
      .listMissions(teamId, effectiveManagerAgentId, undefined, 1, 6)
      .then((items) => {
        if (cancelled) return;
        setWorkspaceMissions(items);
      })
      .catch((error) => {
        if (cancelled) return;
        console.error('Failed to load workspace missions:', error);
      })
      .finally(() => {
        if (!cancelled) setWorkspaceMissionsLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [effectiveManagerAgentId, teamId]);

  const loadWorkspaceDocuments = useCallback(async () => {
    const boundIds = selectedAvatar?.boundDocumentIds || [];
    if (boundIds.length === 0) {
      setWorkspaceDocuments([]);
      setSelectedWorkspaceDocumentId(null);
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
      setSelectedWorkspaceDocumentId((current) => {
        if (current && docs.some((item) => item.id === current)) return current;
        return docs[0]?.id || null;
      });
    } catch (error) {
      console.error('Failed to load workspace documents:', error);
      setWorkspaceDocuments([]);
      setSelectedWorkspaceDocumentId(null);
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
      setPermissionSelectedExtensions([]);
      setPermissionSelectedSkillIds([]);
      setPermissionDocumentAccessMode('read_only');
      return;
    }

    setPermissionSelectedDocIds(selectedAvatar.boundDocumentIds || []);
    setPermissionDocumentAccessMode(selectedAvatar.documentAccessMode || 'read_only');
    setPermissionSelectedExtensions(
      selectedAvatar.allowedExtensions && selectedAvatar.allowedExtensions.length > 0
        ? selectedAvatar.allowedExtensions
        : selectedAvatarRuntimeExtensionOptions.map((item) => item.id),
    );
    setPermissionSelectedSkillIds(
      selectedAvatar.allowedSkillIds && selectedAvatar.allowedSkillIds.length > 0
        ? selectedAvatar.allowedSkillIds
        : selectedAvatarAssignedSkillEntries.map((item) => item.id),
    );
  }, [selectedAvatar, selectedAvatarAssignedSkillEntries, selectedAvatarRuntimeExtensionOptions]);

  useEffect(() => {
    setWorkspaceDocumentMode(defaultWorkspaceDocumentMode(selectedWorkspaceObjectSummary?.kind));
    setWorkspaceDocumentText('');
    setWorkspaceDocumentLock(null);
    setWorkspaceDocumentCompareVersions(null);
  }, [selectedWorkspaceDocumentId, selectedWorkspaceObjectSummary?.kind]);

  useEffect(() => {
    let cancelled = false;
    const targets = workspaceMissions.slice(0, 4);
    if (targets.length === 0) {
      setWorkspaceMissionDetails({});
      setWorkspaceMissionArtifacts({});
      return;
    }
    setWorkspaceMissionMetaLoading(true);
    Promise.all(
      targets.map(async (mission) => {
        const [detail, artifacts] = await Promise.all([
          missionApi.getMission(mission.mission_id).catch(() => null),
          missionApi.listArtifacts(mission.mission_id).catch(() => [] as MissionArtifact[]),
        ]);
        return { missionId: mission.mission_id, detail, artifacts };
      }),
    )
      .then((results) => {
        if (cancelled) return;
        const detailMap: Record<string, MissionDetail> = {};
        const artifactMap: Record<string, MissionArtifact[]> = {};
        for (const item of results) {
          if (item.detail) detailMap[item.missionId] = item.detail;
          artifactMap[item.missionId] = item.artifacts;
        }
        setWorkspaceMissionDetails(detailMap);
        setWorkspaceMissionArtifacts(artifactMap);
      })
      .catch((error) => {
        if (!cancelled) {
          console.error('Failed to load workspace mission meta:', error);
        }
      })
      .finally(() => {
        if (!cancelled) setWorkspaceMissionMetaLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceMissions]);

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

  const visiblePersistedEvents = useMemo(() => {
    const keyword = persistedEventSearch.trim().toLowerCase();
    return persistedEvents.filter((event) => {
      if (persistedEventFilter === 'error' && eventSeverity(event) !== 'error') return false;
      if (persistedEventFilter === 'tool' && !['toolcall', 'toolresult'].includes(event.event_type)) return false;
      if (persistedEventFilter === 'thinking' && !['thinking', 'turn', 'compaction'].includes(event.event_type)) return false;
      if (persistedEventFilter === 'status' && !['status', 'done', 'workspace_changed'].includes(event.event_type)) return false;
      if (!keyword) return true;
      const text = `${event.event_type} ${eventSummary(event)}`.toLowerCase();
      return text.includes(keyword);
    });
  }, [persistedEventFilter, persistedEventSearch, persistedEvents]);

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
          event.actor_name ? `执行人：${event.actor_name}` : '',
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
        const actorMeta = event.actor_name ? [`执行人:${event.actor_name}`] : [];
        const eventMeta = event.event_type ? [`事件:${event.event_type}`] : [];
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
        meta: [item.expectedGain].filter(Boolean),
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
        meta: [item.problemType, item.risk].filter(Boolean),
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
        agentApi.listAgents(teamId, 1, 200),
        avatarPortalApi.listInstances(teamId).catch(() => []),
      ]);
      const avatarItems = (portalRes.items || []).filter(isAvatar);
      const nextAgents = agentRes.items || [];
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
    try {
      setWorkbenchSnapshot(null);
      const [detail, governancePayload, governanceEventPayload, governanceQueuePayload, workbenchPayload] = await Promise.all([
        avatarPortalApi.get(teamId, avatarId),
        avatarPortalApi.getGovernance(teamId, avatarId).catch(() => null),
        avatarPortalApi.listGovernanceEvents(teamId, avatarId, 120).catch(() => []),
        avatarPortalApi.listGovernanceQueue(teamId, avatarId).catch(() => null),
        avatarPortalApi.getWorkbenchSnapshot(teamId, avatarId).catch(() => null),
      ]);
      setSelectedAvatar(detail);
      setGovernanceEvents(governanceEventPayload);
      setGovernanceQueue(governanceQueuePayload || []);
      setGovernanceQueueLoaded(Array.isArray(governanceQueuePayload));
      setWorkbenchSnapshot(workbenchPayload);
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
        setManagerSessionId(saved || null);
      } catch {
        setManagerSessionId(null);
      }
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
      setSelectedAvatar(null);
      setGovernanceEvents([]);
      setGovernanceQueue([]);
      setGovernanceQueueLoaded(false);
      setGovernance(createEmptyGovernanceState());
      setGovernanceConfig(DEFAULT_AUTOMATION_CONFIG);
      setManagerSessionId(null);
      setRuntimeLogFilter('pending');
      setWorkbenchSnapshot(null);
    }
  }, [addToast, teamId, t, sessionStoragePrefix]);

  const handleSaveAvatarPermissions = useCallback(async () => {
    if (!selectedAvatar || !canManage || savingPermissionConfig) {
      return;
    }

    try {
      setSavingPermissionConfig(true);
      const updated = await avatarPortalApi.update(teamId, selectedAvatar.id, {
        boundDocumentIds: permissionSelectedDocIds,
        allowedExtensions: permissionSelectedExtensions,
        allowedSkillIds: permissionSelectedSkillIds,
        documentAccessMode: permissionDocumentAccessMode,
      });
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
    savingPermissionConfig,
    selectedAvatar,
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
    if (selectedAvatar) {
      const managerAgentId =
        selectedAvatar.codingAgentId ||
        selectedAvatar.agentId ||
        selectedAvatar.serviceAgentId ||
        undefined;
      const res = await chatApi.createPortalManagerSession(teamId, managerAgentId);
      try {
        window.localStorage.setItem(`${sessionStoragePrefix}${selectedAvatar.id}`, res.session_id);
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
  }, [selectedAvatar, selectedManagerGroupId, sessionStoragePrefix, t, teamId]);

  const onManagerSessionCreated = useCallback((sessionId: string) => {
    setManagerSessionId(sessionId);
    const key = selectedAvatar
      ? `${sessionStoragePrefix}${selectedAvatar.id}`
      : (selectedManagerGroupId
        ? `${sessionStoragePrefix}__bootstrap:${selectedManagerGroupId}`
        : null);
    if (!key) return;
    try {
      window.localStorage.setItem(key, sessionId);
    } catch {}
  }, [selectedAvatar, selectedManagerGroupId, sessionStoragePrefix]);

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
      )}\n管理Agent固定为: ${effectiveManagerAgentId}。要求新分身归属该管理Agent。`;
    } else if (kind === 'createExternalPartner') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateExternalPartner',
        '请为我创建一个新的对外数字分身，定位为客户/合作伙伴协同入口：先梳理可开放的文档范围、协作边界和常见问题，再调用 create_digital_avatar 创建，并回读 profile 校验，最后给我发布建议。'
      )}\n管理Agent固定为: ${effectiveManagerAgentId}。要求新分身归属该管理Agent。`;
    } else if (kind === 'createInternalKnowledge') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateInternalKnowledge',
        '请为我创建一个新的对内数字分身，定位为知识问答/制度检索助手：先定义内部使用场景、文档边界和回答风格，再调用 create_digital_avatar 创建，并回读 profile 校验，最后输出上线建议。'
      )}\n管理Agent固定为: ${effectiveManagerAgentId}。要求新分身归属该管理Agent。`;
    } else if (kind === 'createInternalOps') {
      text = `${t(
        'digitalAvatar.workspace.quickPromptCreateInternalOps',
        '请为我创建一个新的对内数字分身，定位为流程执行/任务跟进助手：先定义任务目标、触发方式和审批边界，再调用 create_digital_avatar 创建，并回读 profile 校验，最后输出执行建议。'
      )}\n管理Agent固定为: ${effectiveManagerAgentId}。要求新分身归属该管理Agent。`;
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

  const queueManagerPrompt = useCallback((text: string, successMessage?: string) => {
    setManagerComposeRequest({
      id: makeId('workspace_entry'),
      text,
      autoSend: false,
    });
    addToast('success', successMessage || t('digitalAvatar.actions.quickPromptPrepared', '已填入输入框'));
  }, [addToast, t]);

  const handleAskQuestionEntry = useCallback(() => {
    const text = selectedAvatar
      ? t(
          'digitalAvatar.workspace.askQuestionStarterSelected',
          '请围绕当前分身「{{name}}」回答我的问题。如果需要转成长程委托或共同处理，请先明确说明原因和下一步。',
          { name: selectedAvatar.name }
        )
      : t(
          'digitalAvatar.workspace.askQuestionStarter',
          '请先根据我的问题判断，应该直接回答、共同处理，还是转成长程委托。'
        );
    queueManagerPrompt(text, t('digitalAvatar.workspace.entryPrepared', '已准备好入口内容，请确认后发送'));
    setWorkEntryMode('ask');
  }, [queueManagerPrompt, selectedAvatar, t]);

  const handlePlanRoleEntry = useCallback(() => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const text = `${t(
      'digitalAvatar.workspace.planRolePrompt',
      '请先帮我规划一个新的数字岗位：明确服务对象、职责边界、工作方式、文档权限和风险边界；如果信息足够，再继续创建分身。如果信息不足，先只给我最少确认项。'
    )}\n管理Agent固定为: ${effectiveManagerAgentId}。新分身必须归属该管理Agent。`;
    queueManagerPrompt(
      text,
      t('digitalAvatar.workspace.planRolePrepared', '已填入岗位规划请求，请确认后发送'),
    );
    setWorkEntryMode('ask');
  }, [addToast, effectiveManagerAgentId, queueManagerPrompt, t]);

  const handlePrepareAvatarCreation = useCallback(() => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const text = `${t(
      'digitalAvatar.workspace.createAvatarPrompt',
      '请根据我的业务目标，为当前管理组创建一个新分身。先确认应创建对外服务还是对内执行岗位，再补齐运行方式、文档模式、边界和默认能力；如果信息足够，请直接调用 create_digital_avatar 创建，并回读 profile 校验。'
    )}\n管理Agent固定为: ${effectiveManagerAgentId}。如需模板，请优先使用通用Agent模板；不要创建新的管理Agent。`;
    queueManagerPrompt(
      text,
      t('digitalAvatar.workspace.createAvatarPrepared', '已填入分身创建请求，请确认后发送'),
    );
    setWorkEntryMode('ask');
  }, [addToast, effectiveManagerAgentId, queueManagerPrompt, t]);

  const handlePrepareTaskDelegation = useCallback(() => {
    const goal = taskGoalDraft.trim();
    if (!goal) {
      addToast('error', t('digitalAvatar.workspace.taskGoalRequired', '请先写清楚你要交付的任务目标。'));
      return;
    }
    const contextBits = [
      selectedAvatar
        ? t(
            'digitalAvatar.workspace.taskAvatarContext',
            '当前岗位：{{name}}（{{type}}）',
            {
              name: selectedAvatar.name,
              type: selectedAvatarType === 'internal'
                ? t('digitalAvatar.filters.internal')
                : selectedAvatarType === 'external'
                ? t('digitalAvatar.filters.external')
                : t('digitalAvatar.labels.unset'),
            }
          )
        : null,
      selectedAvatarDocumentAccessMode
        ? t('digitalAvatar.workspace.taskDocModeContext', '文档模式：{{mode}}', {
            mode: formatDocumentAccessMode(selectedAvatarDocumentAccessMode, t),
          })
        : null,
      taskOutcomeDraft.trim()
        ? t('digitalAvatar.workspace.taskOutcomeContext', '期望交付：{{outcome}}', {
            outcome: taskOutcomeDraft.trim(),
          })
        : null,
    ].filter(Boolean);

    const text = `${t(
      'digitalAvatar.workspace.taskDelegationPrompt',
      '请把下面的需求整理成可执行方案：先判断适合直接处理、共同处理还是长程委托，再给出执行建议。'
    )}\n\n${t('digitalAvatar.workspace.taskGoalLabel', '任务目标')}：${goal}${contextBits.length > 0 ? `\n${contextBits.join('\n')}` : ''}`;

    queueManagerPrompt(text, t('digitalAvatar.workspace.taskPrepared', '已整理成任务委托草稿，请确认后发送'));
  }, [addToast, queueManagerPrompt, selectedAvatar, selectedAvatarType, t, taskGoalDraft, taskOutcomeDraft]);

  const handlePrepareCollaborate = useCallback(() => {
    const selectedDocs = selectedAvatar?.boundDocumentIds || [];
    const text = selectedAvatar
      ? `${t(
          'digitalAvatar.workspace.collaboratePromptSelected',
          '请围绕当前分身「{{name}}」进入共同处理模式：先明确要处理的对象、修改方式和风险边界，再开始协作。',
          { name: selectedAvatar.name }
        )}\n${t('digitalAvatar.workspace.collaboratePromptDocHint', '当前绑定工作对象数')}：${selectedDocs.length}`
      : t(
          'digitalAvatar.workspace.collaboratePrompt',
          '请先帮我确认要共同处理的对象（文档、页面、PPT 或数据），再给出最合适的协作方式。'
        );
    queueManagerPrompt(text, t('digitalAvatar.workspace.collaboratePrepared', '已填入共同处理请求，请确认后发送'));
  }, [queueManagerPrompt, selectedAvatar, t]);

  const handleOpenDocumentsWorkspace = useCallback(() => {
    navigate(`/teams/${teamId}?section=documents`);
  }, [navigate, teamId]);

  const handleSelectWorkspaceDocument = useCallback(async (docId: string | null) => {
    if (
      workspaceDocumentMode === 'edit'
      && workspaceDocumentLock
      && selectedWorkspaceDocument
      && selectedWorkspaceDocument.id !== docId
    ) {
      try {
        await documentApi.releaseLock(teamId, selectedWorkspaceDocument.id);
      } catch {
        // Ignore lock release failures when switching docs.
      }
      setWorkspaceDocumentLock(null);
    }
    setSelectedWorkspaceDocumentId(docId);
  }, [selectedWorkspaceDocument, teamId, workspaceDocumentLock, workspaceDocumentMode]);

  const handleApplyWorkspaceRecommendation = useCallback(async (recommendation: WorkspaceRecommendation) => {
    if (recommendation.docId) {
      await handleSelectWorkspaceDocument(recommendation.docId);
    }
    setWorkEntryMode(recommendation.mode);
    queueManagerPrompt(
      recommendation.prompt,
      t('digitalAvatar.workspace.recommendationPrepared', {
        defaultValue: '已填入协作建议，请确认后发送。',
      }),
    );
  }, [handleSelectWorkspaceDocument, queueManagerPrompt, t]);

  const handleWorkspaceDocumentEdit = useCallback(async () => {
    if (!selectedWorkspaceDocument || !workspaceDocumentEditable) return;
    setWorkspaceDocumentLoading(true);
    try {
      const [content, lock] = await Promise.all([
        documentApi.getTextContent(teamId, selectedWorkspaceDocument.id),
        documentApi.acquireLock(teamId, selectedWorkspaceDocument.id),
      ]);
      setWorkspaceDocumentText(content.text);
      setWorkspaceDocumentLock(lock);
      setWorkspaceDocumentMode('edit');
    } catch (error) {
      console.error('Failed to prepare workspace document edit:', error);
      addToast('error', t('digitalAvatar.workspace.docEditFailed', '无法进入编辑模式，请稍后重试。'));
    } finally {
      setWorkspaceDocumentLoading(false);
    }
  }, [addToast, selectedWorkspaceDocument, teamId, t, workspaceDocumentEditable]);

  const handleWorkspaceDocumentEditSaved = useCallback(async () => {
    setWorkspaceDocumentMode('preview');
    setWorkspaceDocumentLock(null);
    await loadWorkspaceDocuments();
    addToast('success', t('digitalAvatar.workspace.docSaved', '文档草稿已保存，可继续审阅或交给管理 Agent。'));
  }, [addToast, loadWorkspaceDocuments, t]);

  const handleWorkspaceDocumentEditClosed = useCallback(async () => {
    if (workspaceDocumentLock && selectedWorkspaceDocument) {
      try {
        await documentApi.releaseLock(teamId, selectedWorkspaceDocument.id);
      } catch {
        // Ignore close-time release failures.
      }
    }
    setWorkspaceDocumentLock(null);
    setWorkspaceDocumentMode('preview');
  }, [selectedWorkspaceDocument, teamId, workspaceDocumentLock]);

  const handleWorkspaceDocumentVersions = useCallback(() => {
    if (!selectedWorkspaceDocument) return;
    setWorkspaceDocumentCompareVersions(null);
    setWorkspaceDocumentMode('versions');
  }, [selectedWorkspaceDocument]);

  const handleWorkspaceDocumentCompare = useCallback((v1: VersionSummary, v2: VersionSummary) => {
    setWorkspaceDocumentCompareVersions([v1, v2]);
    setWorkspaceDocumentMode('diff');
  }, []);

  const handleLaunchMission = useCallback(async () => {
    if (!effectiveManagerAgentId) {
      addToast('error', t('digitalAvatar.states.noManagerAgent'));
      return;
    }
    const goal = missionGoalDraft.trim();
    if (!goal) {
      addToast('error', t('digitalAvatar.workspace.missionGoalRequired', '请先填写长程委托目标。'));
      return;
    }

    const avatarContext = selectedAvatar
      ? [
          t('digitalAvatar.workspace.missionContextAvatar', '当前岗位：{{name}}', { name: selectedAvatar.name }),
          t(
            'digitalAvatar.workspace.missionContextType',
            '分身类型：{{type}}',
            {
              type: selectedAvatarType === 'internal'
                ? t('digitalAvatar.filters.internal')
                : selectedAvatarType === 'external'
                ? t('digitalAvatar.filters.external')
                : t('digitalAvatar.labels.unset'),
            }
          ),
          selectedAvatar.documentAccessMode
            ? t('digitalAvatar.workspace.missionContextDocMode', '文档模式：{{mode}}', {
                mode: selectedAvatar.documentAccessMode,
              })
            : null,
          selectedAvatar.boundDocumentIds?.length
            ? t('digitalAvatar.workspace.missionContextBoundDocs', '已绑定文档：{{count}} 项', {
                count: selectedAvatar.boundDocumentIds.length,
              })
            : null,
        ].filter(Boolean).join('\n')
      : '';

    const context = [
      avatarContext,
      missionContextDraft.trim(),
      t(
        'digitalAvatar.workspace.missionContextRule',
        '请以管理 Agent 身份规划执行，必要时调用内部执行能力，并在过程中保留审计与交付物。'
      ),
    ]
      .filter(Boolean)
      .join('\n\n');

    setCreatingMission(true);
    try {
      const created = await missionApi.createMission({
        agent_id: effectiveManagerAgentId,
        goal,
        context,
        route_mode: 'mission',
        approval_policy: 'auto',
        execution_mode: 'sequential',
        execution_profile: 'auto',
        attached_document_ids: selectedAvatar?.boundDocumentIds || [],
      });

      if (!created.mission_id) {
        throw new Error(created.message || t('digitalAvatar.workspace.missionCreateFailed', '未能创建长程委托。'));
      }

      await missionApi.startMission(created.mission_id);
      setWorkspaceMissions((prev) => {
        const optimistic: MissionListItem = {
          mission_id: created.mission_id!,
          agent_id: effectiveManagerAgentId,
          agent_name: getAgentDisplayName(managerAgent, t('digitalAvatar.labels.managerAgent')),
          goal,
          status: 'running',
          approval_policy: 'auto',
          step_count: 0,
          completed_steps: 0,
          current_step: 0,
          total_tokens_used: 0,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          execution_mode: 'sequential',
          execution_profile: 'auto',
          resolved_execution_profile: 'auto',
          goal_count: 0,
          completed_goals: 0,
          pivots: 0,
          attached_doc_count: selectedAvatar?.boundDocumentIds?.length || 0,
        };
        return [optimistic, ...prev.filter((item) => item.mission_id !== optimistic.mission_id)].slice(0, 6);
      });
      addToast('success', t('digitalAvatar.workspace.missionStarted', '长程委托已启动'));
      navigate(`/teams/${teamId}/missions/${created.mission_id}`);
    } catch (err) {
      addToast('error', err instanceof Error ? err.message : t('common.error'));
    } finally {
      setCreatingMission(false);
    }
  }, [
    addToast,
    effectiveManagerAgentId,
    missionContextDraft,
    missionGoalDraft,
    navigate,
    selectedAvatar,
    selectedAvatarType,
    managerAgent?.name,
    t,
    teamId,
  ]);

  const handleOpenMissionDetail = useCallback((missionId: string) => {
    navigate(`/teams/${teamId}/missions/${missionId}`);
  }, [navigate, teamId]);

  const handleResumeWorkspaceMission = useCallback(async (missionId: string) => {
    try {
      const result = await missionApi.resumeMission(missionId);
      addToast(
        'success',
        result.status === 'already_running'
          ? t('digitalAvatar.workspace.missionAlreadyRunning', '该长程委托已在运行')
          : t('digitalAvatar.workspace.missionResumed', '长程委托已继续执行'),
      );
      navigate(`/teams/${teamId}/missions/${missionId}`);
    } catch (error) {
      addToast('error', error instanceof Error ? error.message : t('common.error'));
    }
  }, [addToast, navigate, t, teamId]);

  const openWorkbenchAction = useCallback((
    actionKind?: string | null,
    actionTargetId?: string | null,
  ): { label?: string; action?: () => void } => {
    switch (actionKind) {
      case 'open_mission':
        return actionTargetId
          ? {
              label: t('digitalAvatar.workspace.openMissionDetail', '查看进度'),
              action: () => handleOpenMissionDetail(actionTargetId),
            }
          : {};
      case 'resume_mission':
        return actionTargetId
          ? {
              label: t('digitalAvatar.workspace.resumeMission', '继续处理'),
              action: () => {
                void handleResumeWorkspaceMission(actionTargetId);
              },
            }
          : {};
      case 'open_logs':
        return {
          label: t('digitalAvatar.workspace.openLogsConsole', '打开日志台'),
          action: () => {
            setInspectorOpen(true);
            setInspectorTab('logs');
          },
        };
      case 'open_governance':
        return {
          label: t('digitalAvatar.workspace.openGovernanceConsole', '打开治理台'),
          action: () => {
            setInspectorOpen(true);
            setInspectorTab('governance');
          },
        };
      default:
        return {};
    }
  }, [handleOpenMissionDetail, handleResumeWorkspaceMission, t]);

  const managerReportRows = useMemo(() => {
    if (workbenchSnapshot) {
      return workbenchSnapshot.reports.map((row) => {
        const { label, action } = openWorkbenchAction(row.action_kind, row.action_target_id);
        return {
          id: row.id,
          ts: row.ts,
          kind: (['delivery', 'progress', 'runtime', 'governance'].includes(row.kind)
            ? row.kind
            : 'governance') as ManagerReportKind,
          title: row.title,
          summary: row.summary,
          status: row.status,
          source: row.source,
          recommendation: row.recommendation || undefined,
          workObjects: row.work_objects || [],
          outputs: row.outputs || [],
          needsDecision: Boolean(row.needs_decision),
          actionLabel: label,
          action,
        };
      });
    }

    const rows: Array<{
      id: string;
      ts: string;
      kind: ManagerReportKind;
      title: string;
      summary: string;
      status: string;
      source: string;
      recommendation?: string;
      workObjects: string[];
      outputs: string[];
      needsDecision: boolean;
      actionLabel?: string;
      action?: () => void;
    }> = [];

    if (latestCompletedWorkspaceMission) {
      rows.push({
        id: `mission-delivery:${latestCompletedWorkspaceMission.mission_id}`,
        ts: latestCompletedWorkspaceMission.updated_at || latestCompletedWorkspaceMission.created_at,
        kind: 'delivery',
        title: latestCompletedWorkspaceMission.goal,
        summary:
          latestCompletedWorkspaceMissionDetail?.final_summary
          || t(
            'digitalAvatar.workspace.latestDeliveryFallback',
            '这次长程委托已经完成，摘要与交付物会优先展示在这里，方便继续审阅或再次交付。',
          ),
        status: latestCompletedWorkspaceMission.status,
        source:
          getAgentName(
            agents,
            selectedAvatarServiceAgentId,
            selectedAvatar?.name || t('digitalAvatar.workspace.reportSourceAvatar', { defaultValue: '当前岗位' }),
          ),
        recommendation: t(
          'digitalAvatar.workspace.reportRecommendationReviewDelivery',
          { defaultValue: '先审阅交付结果，再决定是直接采用、继续优化，还是回到管理 Agent 追加要求。' },
        ),
        workObjects: workspaceObjectSummaries
          .slice(0, 3)
          .map(({ doc }) => doc.display_name || doc.name)
          .filter(Boolean),
        outputs: [],
        needsDecision: false,
        actionLabel: t('digitalAvatar.workspace.openMissionDetail', '查看进度'),
        action: () => handleOpenMissionDetail(latestCompletedWorkspaceMission.mission_id),
      });
    }

    if (latestAttentionWorkspaceMission) {
      rows.push({
        id: `mission-progress:${latestAttentionWorkspaceMission.mission_id}`,
        ts: latestAttentionWorkspaceMission.updated_at || latestAttentionWorkspaceMission.created_at,
        kind: 'progress',
        title: latestAttentionWorkspaceMission.goal,
        summary:
          latestAttentionWorkspaceMission.status === 'running'
            ? t(
              'digitalAvatar.workspace.attentionRunningHint',
              '这项长程委托正在处理中。你可以直接查看进度，或等待管理 Agent 汇总后继续推进。',
            )
            : t(
              'digitalAvatar.workspace.attentionRecoverHint',
              '这项长程委托目前需要继续跟进。建议先查看进度，必要时继续处理或回到管理 Agent 对话调整方案。',
            ),
        status: latestAttentionWorkspaceMission.status,
        source:
          getAgentName(
            agents,
            selectedAvatarServiceAgentId,
            selectedAvatar?.name || t('digitalAvatar.workspace.reportSourceAvatar', { defaultValue: '当前岗位' }),
          ),
        recommendation:
          latestAttentionWorkspaceMission.status === 'running'
            ? t(
              'digitalAvatar.workspace.reportRecommendationWatchMission',
              { defaultValue: '先查看执行进度；如果边界、对象或交付目标需要变化，再回到管理 Agent 对话调整。' },
            )
            : t(
              'digitalAvatar.workspace.reportRecommendationResumeMission',
              { defaultValue: '这项任务需要管理 Agent 继续决策；建议先查看原因，再决定继续处理还是修改方案。' },
            ),
        workObjects: workspaceObjectSummaries
          .slice(0, 3)
          .map(({ doc }) => doc.display_name || doc.name)
          .filter(Boolean),
        outputs: [],
        needsDecision: ['failed', 'paused'].includes(latestAttentionWorkspaceMission.status),
        actionLabel:
          latestAttentionWorkspaceMission.status === 'failed' || latestAttentionWorkspaceMission.status === 'paused'
            ? t('digitalAvatar.workspace.resumeMission', '继续处理')
            : t('digitalAvatar.workspace.openMissionDetail', '查看进度'),
        action:
          latestAttentionWorkspaceMission.status === 'failed' || latestAttentionWorkspaceMission.status === 'paused'
            ? () => {
                void handleResumeWorkspaceMission(latestAttentionWorkspaceMission.mission_id);
              }
            : () => handleOpenMissionDetail(latestAttentionWorkspaceMission.mission_id),
      });
    }

    governanceTimelineRows.slice(0, 4).forEach((row) => {
      rows.push({
        id: `governance-report:${row.id}`,
        ts: row.ts,
        kind: row.rowType === 'runtime' ? 'runtime' : 'governance',
        title: row.title,
        summary:
          row.detail
          || t(
            'digitalAvatar.workspace.managerReportFallback',
            '本次处理已经写入治理与审计记录，可继续查看详情。',
          ),
        status: row.status,
        source:
          row.meta.find((item) => item.startsWith('执行人:'))?.replace('执行人:', '')
          || selectedAvatar?.name
          || t('digitalAvatar.workspace.reportSourceSystem', { defaultValue: '系统汇总' }),
        recommendation:
          row.rowType === 'runtime'
            ? t(
              'digitalAvatar.workspace.reportRecommendationRuntime',
              { defaultValue: '如果涉及失败恢复或对象补充，建议先打开治理台，再决定是否继续执行。' },
            )
            : t(
              'digitalAvatar.workspace.reportRecommendationGovernance',
              { defaultValue: '如果需要进一步确认权限、提案或策略，先打开治理台查看再决定是否执行。' },
            ),
        workObjects: workspaceObjectSummaries
          .slice(0, 2)
          .map(({ doc }) => doc.display_name || doc.name)
          .filter(Boolean),
        outputs: [],
        needsDecision: ['failed', 'pending', 'review'].includes(row.status),
        actionLabel: t('digitalAvatar.workspace.openGovernanceConsole', '打开治理台'),
        action: () => {
          setInspectorOpen(true);
          setInspectorTab(row.rowType === 'runtime' ? 'logs' : 'governance');
        },
      });
    });

    return rows
      .sort((a, b) => Date.parse(b.ts) - Date.parse(a.ts))
      .filter((item, index, list) => list.findIndex((candidate) => candidate.id === item.id) === index)
      .slice(0, 4);
  }, [
    governanceTimelineRows,
    handleOpenMissionDetail,
    handleResumeWorkspaceMission,
    agents,
    latestAttentionWorkspaceMission,
    latestCompletedWorkspaceMission,
    latestCompletedWorkspaceMissionDetail?.final_summary,
    selectedAvatar?.name,
    selectedAvatarServiceAgentId,
    t,
    workbenchSnapshot,
    workspaceObjectSummaries,
    openWorkbenchAction,
  ]);

  const decisionWorkbenchItems = useMemo(() => {
    if (workbenchSnapshot) {
      return workbenchSnapshot.decisions.map((item) => {
        const { label, action } = openWorkbenchAction(item.action_kind, item.action_target_id);
        return {
          id: item.id,
          kind: item.kind || 'governance',
          title: item.title,
          detail: item.detail,
          status: item.status,
          risk: normalizeGovernanceRisk(item.risk),
          source: item.source || t('digitalAvatar.workspace.reportSourceSystem', { defaultValue: '系统汇总' }),
          recommendation: item.recommendation || undefined,
          workObjects: item.work_objects || [],
          actionLabel: label || t('digitalAvatar.workspace.openGovernanceConsole', '打开治理台'),
          action: action || (() => {
            setInspectorOpen(true);
            setInspectorTab('governance');
          }),
        };
      });
    }

    const rows: Array<{
      id: string;
      kind: string;
      title: string;
      detail: string;
      status: string;
      risk: GovernanceRiskFilter;
      source: string;
      recommendation?: string;
      workObjects: string[];
      actionLabel: string;
      action: () => void;
    }> = [];

    if (latestAttentionWorkspaceMission && ['failed', 'paused'].includes(latestAttentionWorkspaceMission.status)) {
      rows.push({
        id: `decision-mission:${latestAttentionWorkspaceMission.mission_id}`,
        kind: 'mission',
        title: latestAttentionWorkspaceMission.goal,
        detail: t(
          'digitalAvatar.workspace.decisionMissionHint',
          '这项长程委托当前需要管理 Agent 继续决策或恢复执行。',
        ),
        status: latestAttentionWorkspaceMission.status,
        risk: 'medium',
        source:
          getAgentName(
            agents,
            selectedAvatarServiceAgentId,
            selectedAvatar?.name || t('digitalAvatar.workspace.reportSourceAvatar', { defaultValue: '当前岗位' }),
          ),
        recommendation: t(
          'digitalAvatar.workspace.decisionMissionRecommendation',
          '先查看这次长程委托的进度和原因，再决定继续处理、调整边界，还是回到管理对话重新规划。',
        ),
        workObjects: workspaceObjectSummaries
          .slice(0, 3)
          .map(({ doc }) => doc.display_name || doc.name)
          .filter(Boolean),
        actionLabel: t('digitalAvatar.workspace.resumeMission', '继续处理'),
        action: () => {
          void handleResumeWorkspaceMission(latestAttentionWorkspaceMission.mission_id);
        },
      });
    }

    pendingGovernanceQueueItems.slice(0, 4).forEach((item) => {
      rows.push({
        id: `decision-queue:${item.id}`,
        kind: item.kind,
        title: item.title,
        detail:
          item.detail
          || t(
            'digitalAvatar.workspace.decisionQueueFallback',
            '这项治理建议已经进入待决策队列，建议先阅读内容再决定是否执行。',
          ),
        status: item.status,
        risk: extractRiskFromTexts(item.meta),
        source:
          item.meta.find((entry) => !entry.includes('风险')) ||
          t('digitalAvatar.workspace.reportSourceSystem', { defaultValue: '系统汇总' }),
        recommendation:
          item.kind === 'capability'
            ? t('digitalAvatar.workspace.decisionCapabilityRecommendation', '先判断是否需要放开当前能力边界，再决定批准、试运行或继续收敛。')
            : item.kind === 'proposal'
              ? t('digitalAvatar.workspace.decisionProposalRecommendation', '先判断这项岗位提案是否值得试运行，再决定批准、试点或拒绝。')
              : t('digitalAvatar.workspace.decisionTicketRecommendation', '先确认优化证据和预期收益，再决定试运行、批准或继续观察。'),
        workObjects: workspaceObjectSummaries
          .slice(0, 2)
          .map(({ doc }) => doc.display_name || doc.name)
          .filter(Boolean),
        actionLabel: t('digitalAvatar.workspace.openGovernanceConsole', '打开治理台'),
        action: () => {
          setInspectorOpen(true);
          setInspectorTab('governance');
        },
      });
    });

    return rows
      .filter((item, index, list) => list.findIndex((candidate) => candidate.id === item.id) === index)
      .slice(0, 4);
  }, [
    agents,
    handleResumeWorkspaceMission,
    latestAttentionWorkspaceMission,
    openWorkbenchAction,
    pendingGovernanceQueueItems,
    selectedAvatar?.name,
    selectedAvatarServiceAgentId,
    t,
    workbenchSnapshot,
    workspaceObjectSummaries,
  ]);

  const hasWorkbenchContent =
    managerReportRows.length > 0 || decisionWorkbenchItems.length > 0 || workspaceObjectSummaries.length > 0;
  const workspacePanelSummaryText = useMemo(() => {
    if (selectedAvatar) {
      return hasWorkbenchContent
        ? t(
            'digitalAvatar.workspace.panelSummarySelected',
            '岗位汇报、待决策事项、工作对象和长程委托都已收进工作面板，主区只保留管理 Agent 对话。'
          )
        : t(
            'digitalAvatar.workspace.panelSummarySelectedEmpty',
            '当前岗位还没有新的汇报或工作对象。先和管理 Agent 明确需求，再通过工作面板选择问答、任务或共同处理。'
          );
    }

    if (showWorkspaceBootstrapOnly) {
      return t(
        'digitalAvatar.workspace.panelSummaryBootstrap',
        '当前管理组还没有岗位。先让管理 Agent 规划岗位，再在工作面板里继续创建和治理。'
      );
    }

    return t(
      'digitalAvatar.workspace.panelSummaryGroup',
      '先选择一个岗位，再通过工作面板查看汇报、发起任务、共同处理对象或进入长程委托。'
    );
  }, [hasWorkbenchContent, selectedAvatar, showWorkspaceBootstrapOnly, t]);

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
    try {
      await navigator.clipboard.writeText(text);
      addToast('success', t('digitalAvatar.actions.copiedPrompt', '已复制'));
    } catch {
      addToast('error', t('common.error'));
    }
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
    const structuredPrompt = `${prompt}\n\n请在最终回复末尾严格追加一个结构化回执（不要解释，不要省略）：\n<governance_action_result>{\"action_id\":\"${requestId}\",\"outcome\":\"success|partial|failed\",\"summary\":\"一句话结果摘要\",\"reason\":\"失败/部分成功原因；成功可留空\"}</governance_action_result>`;
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

  const openLaboratory = () => {
    navigate(`/admin/teams/${teamId}?section=laboratory`);
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

  const toggleInspectorPanel = useCallback(() => {
    setFocusMode(false);
    setInspectorOpen((prev) => !prev);
  }, []);

  const openInspectorPanel = useCallback((nextTab?: InspectorTab) => {
    if (nextTab) setInspectorTab(nextTab);
    setFocusMode(false);
    setInspectorOpen(true);
  }, []);

  const toggleFocusMode = useCallback(() => {
    setWorkspacePanelOpen(false);
    setFocusMode((prev) => {
      setInspectorOpen(prev ? true : false);
      return !prev;
    });
  }, []);

  const handleRoleOverviewPrimaryAction = useCallback(() => {
    switch (roleOverviewPrimaryAction.kind) {
      case 'open_mission':
        handleOpenMissionDetail(roleOverviewPrimaryAction.missionId);
        break;
      case 'resume_mission':
        void handleResumeWorkspaceMission(roleOverviewPrimaryAction.missionId);
        break;
      case 'collaborate':
        setWorkEntryMode('collaborate');
        if (roleOverviewPrimaryAction.docId) {
          void handleSelectWorkspaceDocument(roleOverviewPrimaryAction.docId);
        }
        setWorkspacePanelOpen(true);
        break;
      case 'task':
        setWorkEntryMode('task');
        setWorkspacePanelOpen(true);
        break;
      case 'select_avatar':
        setSelectedAvatarId(roleOverviewPrimaryAction.avatarId);
        break;
      case 'create_avatar':
        handlePrepareAvatarCreation();
        break;
    }
  }, [
    handleOpenMissionDetail,
    handlePrepareAvatarCreation,
    handleResumeWorkspaceMission,
    handleSelectWorkspaceDocument,
    roleOverviewPrimaryAction,
  ]);

  return (
    <div className="h-[calc(100vh-40px)] flex flex-col gap-3 overflow-x-hidden overflow-y-auto p-3 sm:p-4">
      <div className={focusMode ? '' : 'rounded-xl border bg-card p-3 sm:p-4'}>
        {focusMode ? (
          <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-card px-3 py-2">
            <div className="flex min-w-0 items-center gap-2 overflow-hidden">
              <div className="flex h-7 w-7 items-center justify-center rounded-md bg-primary/10 text-primary shrink-0">
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
                {selectedAvatar ? (
                  <>
                    <AvatarTypeBadge type={selectedAvatarType} className="shrink-0" />
                    <span className="truncate text-muted-foreground">{selectedAvatar.name}</span>
                    <span className="shrink-0 rounded-full border border-border/60 bg-muted/35 px-2 py-0 text-[10px] text-muted-foreground">
                      {selectedAvatarStatusLabel}
                    </span>
                  </>
                ) : (
                  <span className="shrink-0 rounded-full border border-border/60 bg-muted/35 px-2 py-0 text-[10px] text-muted-foreground">
                    {t('digitalAvatar.states.noAvatarSelected')}
                  </span>
                )}
              </div>
            </div>
            <Button size="sm" className="h-8 px-3" onClick={toggleFocusMode}>
              {t('digitalAvatar.actions.exitFocus', '退出专注')}
            </Button>
          </div>
        ) : (
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-2.5 min-w-0">
              <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-primary/10 text-primary shrink-0">
                <UserRound className="h-4.5 w-4.5" />
              </div>
              <div className="min-w-0">
                <h2 className="text-sm font-semibold truncate">{t('digitalAvatar.title')}</h2>
                <p className="text-caption text-muted-foreground truncate">{t('digitalAvatar.description')}</p>
                <p className="text-[11px] text-muted-foreground/90 truncate">
                  {t(
                    'digitalAvatar.upgradeHint',
                    '适合快速开放标准岗位；如需自定义 SDK、Portal 页面或在线编排，再升级到生态协作高级方案。'
                  )}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="outline" size="sm" onClick={() => setTab(tab === 'workspace' ? 'guide' : 'workspace')}>
                {tab === 'workspace' ? t('digitalAvatar.tabs.guide') : t('digitalAvatar.tabs.workspace')}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => navigate(`/teams/${teamId}/digital-avatars/overview`)}
              >
                {t('digitalAvatar.actions.overview', '治理总览')}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => navigate(`/teams/${teamId}/digital-avatars/audit`)}
              >
                {t('digitalAvatar.actions.auditCenter', { defaultValue: '审计中心' })}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => navigate(`/teams/${teamId}/digital-avatars/policies`)}
              >
                {t('digitalAvatar.actions.policyCenter', '风险策略')}
              </Button>
              <Button variant="outline" size="sm" onClick={() => loadAvatars(false)} disabled={refreshing}>
                {refreshing ? <Loader2 className="w-3.5 h-3.5 mr-1 animate-spin" /> : <RefreshCw className="w-3.5 h-3.5 mr-1" />}
                {t('digitalAvatar.actions.refresh')}
              </Button>
              {canManage && (
                <>
                  <Button variant="outline" size="sm" onClick={() => setCreateManagerOpen(true)}>
                    <Users className="w-3.5 h-3.5 mr-1" />
                    {t('digitalAvatar.actions.createManagerGroup', '新建管理组')}
                  </Button>
                  <Button
                    size="sm"
                    onClick={() => {
                      if (!selectedManagerGroupId) {
                        addToast('error', t('digitalAvatar.states.noManagerAgent'));
                        return;
                      }
                      setCreateOpen(true);
                    }}
                  >
                    <Plus className="w-3.5 h-3.5 mr-1" />
                    {t('digitalAvatar.actions.createAvatarAdvanced', '高级创建分身')}
                  </Button>
                </>
              )}
            </div>
          </div>
        )}
      </div>

      {tab === 'guide' ? (
        <div className="min-h-0 flex-1 overflow-y-auto rounded-xl border bg-card">
          <DigitalAvatarGuide
            teamId={teamId}
            currentAvatarId={selectedAvatar?.id ?? null}
            canSendCommand={Boolean(effectiveManagerAgentId)}
            onCopyCommand={copyGuideCommand}
            onSendCommand={sendGuideCommandToManager}
          />
        </div>
      ) : (
        <>
        {!focusMode && (
          workspaceChromeCollapsed ? (
            <div className="rounded-xl border border-primary/15 bg-card/95 px-3 py-2">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="min-w-0 flex flex-1 items-center gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                    <Users className="h-4 w-4" />
                  </div>
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-[12px] font-semibold text-foreground">
                        {getAgentName(agents, effectiveManagerAgentId, t('digitalAvatar.states.noManagerAgent'))}
                      </span>
                      {managerAgent ? (
                        <AgentTypeBadge
                          type={resolveAgentVisualType(managerAgent)}
                          className="h-5 px-1.5 text-[10px]"
                        />
                      ) : null}
                      {selectedAvatar ? (
                        <>
                          <span className="text-border">·</span>
                          <span className="truncate text-[12px] font-medium text-foreground">
                            {selectedAvatar.name}
                          </span>
                          <AvatarTypeBadge type={selectedAvatarType} className="h-5 px-1.5 text-[10px]" />
                          <span className={`shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] ${avatarStatusBadgeClass(normalizeAvatarStatus(selectedAvatar))}`}>
                            {selectedAvatarStatusLabel}
                          </span>
                        </>
                      ) : (
                        <span className="rounded-full border border-border/60 bg-muted/25 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                          {t('digitalAvatar.states.noAvatarSelected')}
                        </span>
                      )}
                    </div>
                    <p className="mt-0.5 truncate text-[11px] text-muted-foreground">
                      {managerScopeSummaryText}
                    </p>
                  </div>
                </div>
                <div className="flex shrink-0 flex-wrap items-center gap-1.5">
                  <SummaryPill label={t('digitalAvatar.list.title', '分身列表')} value={String(managerGroupStats.total)} accent />
                  <SummaryPill label={t('digitalAvatar.filters.external')} value={String(managerGroupStats.external)} />
                  <SummaryPill label={t('digitalAvatar.filters.internal')} value={String(managerGroupStats.internal)} />
                  <SummaryPill label={t('digitalAvatar.workspace.summaryStatus', '待处理')} value={String(managerGroupStats.pending)} accent={managerGroupStats.pending > 0} />
                </div>
              </div>
            </div>
          ) : (
            <Card className="border-primary/15 bg-card/95">
              <CardContent className="space-y-3 py-3">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex flex-1 items-start gap-3">
                    <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
                      <Users className="h-4.5 w-4.5" />
                    </div>
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="text-sm font-semibold text-foreground">
                          {getAgentName(agents, effectiveManagerAgentId, t('digitalAvatar.states.noManagerAgent'))}
                        </p>
                        {managerAgent ? <AgentTypeBadge type={resolveAgentVisualType(managerAgent)} /> : null}
                      </div>
                      <p className="mt-1 text-[12px] leading-5 text-muted-foreground">
                        {managerScopeSummaryText}
                      </p>
                    </div>
                  </div>
                  <div className="flex shrink-0 flex-wrap items-center gap-2">
                    <SummaryPill label={t('digitalAvatar.list.title', '分身列表')} value={String(managerGroupStats.total)} accent />
                    <SummaryPill label={t('digitalAvatar.filters.external')} value={String(managerGroupStats.external)} />
                    <SummaryPill label={t('digitalAvatar.filters.internal')} value={String(managerGroupStats.internal)} />
                    <SummaryPill label={t('digitalAvatar.status.published', '已发布')} value={String(managerGroupStats.published)} />
                    <SummaryPill
                      label={t('digitalAvatar.labels.pendingCount', '待处理')}
                      value={String(managerGroupStats.pending)}
                      accent={managerGroupStats.pending > 0}
                    />
                  </div>
                </div>

                {managerPreviewAvatars.length > 0 && (
                  <div className="space-y-1.5">
                    <p className="text-[11px] font-medium text-muted-foreground">
                      {t('digitalAvatar.workspace.managerScopeList', '当前管理组分身')}
                    </p>
                    <div className="flex flex-wrap gap-1.5">
                      {managerPreviewAvatars.map((avatar) => {
                        const previewType = detectAvatarType(avatar, avatarProjectionMap[avatar.id]);
                        const previewStatus = normalizeAvatarStatus(avatar);
                        return (
                          <button
                            key={avatar.id}
                            type="button"
                            onClick={() => setSelectedAvatarId(avatar.id)}
                            className={`inline-flex min-w-0 items-center gap-1.5 rounded-full border px-2 py-1 text-[11px] transition-colors ${
                              avatar.id === selectedAvatarId
                                ? 'border-primary/50 bg-primary/10 text-primary'
                                : 'border-border/60 bg-muted/20 text-muted-foreground hover:text-foreground'
                            }`}
                          >
                            <span className="max-w-[180px] truncate">{avatar.name}</span>
                            <AvatarTypeBadge type={previewType} className="h-4.5 px-1 text-[10px]" />
                            <span className={`rounded-full border px-1 py-0 text-[9px] ${avatarStatusBadgeClass(previewStatus)}`}>
                              {avatarStatusLabel(previewStatus, t)}
                            </span>
                          </button>
                        );
                      })}
                      {managerGroupStats.total > managerPreviewAvatars.length && (
                        <span className="inline-flex items-center rounded-full border border-border/60 bg-muted/20 px-2 py-1 text-[11px] text-muted-foreground">
                          +{managerGroupStats.total - managerPreviewAvatars.length}
                        </span>
                      )}
                    </div>
                  </div>
                )}

                <div className="rounded-xl border border-border/60 bg-muted/15 px-3 py-2.5">
                  {!selectedAvatar ? (
                    <div className="flex flex-wrap items-center justify-between gap-3">
                      <div className="min-w-0">
                        <p className="text-[11px] font-medium text-muted-foreground">
                          {t('digitalAvatar.workspace.currentSelection', '当前选中分身')}
                        </p>
                        <p className="mt-0.5 text-sm font-medium text-foreground">
                          {t('digitalAvatar.states.noAvatarSelected')}
                        </p>
                        <p className="mt-1 text-[12px] text-muted-foreground">
                          {t(
                            'digitalAvatar.workspace.currentSelectionHint',
                            '请先从左侧选择一个岗位，或让管理 Agent 先规划并创建一个岗位后，再继续治理、交付或发布。'
                          )}
                        </p>
                      </div>
                      <div className="flex shrink-0 flex-wrap items-center gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => navigate(`/teams/${teamId}/digital-avatars/overview`)}
                        >
                          {t('digitalAvatar.actions.overview', '治理总览')}
                        </Button>
                        {canManage && (
                          <Button
                            size="sm"
                            onClick={() => {
                              if (!selectedManagerGroupId) {
                                addToast('error', t('digitalAvatar.states.noManagerAgent'));
                                return;
                              }
                              handlePlanRoleEntry();
                            }}
                          >
                            <Sparkles className="w-3.5 h-3.5 mr-1" />
                            {t('digitalAvatar.actions.planRole', '发起岗位规划')}
                          </Button>
                        )}
                      </div>
                    </div>
                  ) : (
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="min-w-0 flex flex-1 items-start gap-3">
                        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
                          <UserRound className="h-4.5 w-4.5" />
                        </div>
                        <div className="min-w-0">
                          <p className="text-[11px] font-medium text-muted-foreground">
                            {t('digitalAvatar.workspace.currentSelection', '当前选中分身')}
                          </p>
                          <div className="mt-0.5 flex flex-wrap items-center gap-2">
                            <p className="truncate text-sm font-semibold text-foreground">
                              {selectedAvatar.name}
                            </p>
                            <AvatarTypeBadge type={selectedAvatarType} />
                            <span className={`shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] ${avatarStatusBadgeClass(normalizeAvatarStatus(selectedAvatar))}`}>
                              {selectedAvatarStatusLabel}
                            </span>
                          </div>
                          <p className="mt-1 line-clamp-2 text-[12px] leading-5 text-muted-foreground">
                            {selectedAvatarSummaryText}
                          </p>
                        </div>
                      </div>
                      <div className="grid min-w-[260px] gap-x-4 gap-y-2 text-[11px] text-muted-foreground sm:grid-cols-2">
                        <div>
                          <p>{t('digitalAvatar.labels.serviceAgent')}</p>
                          <p className="mt-0.5 font-medium text-foreground">
                            {getAgentName(agents, selectedAvatarServiceAgentId, t('digitalAvatar.states.noAvatarSelected'))}
                          </p>
                        </div>
                        <div>
                          <p>{t('digitalAvatar.workspace.summaryAccess', '文档模式')}</p>
                          <p className="mt-0.5 font-medium text-foreground">
                            {formatDocumentAccessMode(selectedAvatarDocumentAccessMode, t)}
                          </p>
                        </div>
                        <div>
                          <p>{t('digitalAvatar.workspace.summaryPublish', '发布地址')}</p>
                          <p className="mt-0.5 font-medium text-foreground">
                            {publishPath || t('digitalAvatar.workspace.unpublished', '未发布')}
                          </p>
                        </div>
                        <div>
                          <p>{t('digitalAvatar.list.lastActivity', '最近活动')}</p>
                          <p className="mt-0.5 font-medium text-foreground">
                            {selectedAvatarLastActivityLabel}
                          </p>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          )
        )}
        <div className={`min-h-0 flex-1 grid gap-3 ${focusMode ? 'grid-cols-1' : 'grid-cols-1 lg:grid-cols-[232px_minmax(0,1fr)]'}`}>
          {!focusMode && (
          <Card className="min-h-0 flex flex-col">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm flex items-center justify-between">
                <span>{t('digitalAvatar.list.title')}</span>
                <span className="text-caption font-normal text-muted-foreground">{visibleAvatars.length}</span>
              </CardTitle>
              <div className="space-y-1.5">
                <p className="text-[11px] text-muted-foreground">
                  {t('digitalAvatar.labels.managerGroup', '管理 Agent 组')}
                </p>
                {managerGroupOptions.length === 0 ? (
                  <div className="space-y-2">
                    <div className="rounded-md border border-dashed px-2 py-2 text-[11px] text-muted-foreground">
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
                    <select
                      className="h-8 w-full rounded-md border bg-background px-2 text-xs"
                      value={selectedManagerGroupId || ''}
                      onChange={(e) => {
                        setSelectedAvatarId(null);
                        setBootstrapManagerAgentId(e.target.value);
                      }}
                    >
                      {managerGroupOptions.map((agent) => (
                        <option key={agent.id} value={agent.id}>
                          {getAgentDisplayName(agent, agent.name)}
                        </option>
                      ))}
                    </select>
                  </>
                )}
              </div>
              <div className="grid grid-cols-3 gap-1 rounded-xl border border-border/60 bg-muted/20 p-1">
                {([
                  { key: 'all', label: t('digitalAvatar.filters.allShort', '全部') },
                  { key: 'external', label: t('digitalAvatar.filters.externalShort', '外部') },
                  { key: 'internal', label: t('digitalAvatar.filters.internalShort', '内部') },
                ] as { key: AvatarFilter; label: string }[]).map((item) => (
                  <button
                    key={item.key}
                    className={`flex h-8 min-w-0 items-center justify-center rounded-lg px-1 text-[11px] font-medium leading-none whitespace-nowrap transition-colors ${
                      filter === item.key
                        ? 'bg-background text-foreground shadow-sm'
                        : 'text-muted-foreground hover:bg-background/60 hover:text-foreground'
                    }`}
                    onClick={() => setFilter(item.key)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            </CardHeader>
            <CardContent className="min-h-0 flex-1 overflow-y-auto space-y-3">
              {loading ? (
                <div className="h-28 flex items-center justify-center text-muted-foreground text-caption">
                  <Loader2 className="w-4 h-4 animate-spin mr-1.5" />
                  {t('digitalAvatar.states.loading')}
                </div>
              ) : visibleAvatars.length === 0 ? (
                <div className="rounded-lg border border-dashed p-3 text-caption text-muted-foreground">
                  <p className="font-medium text-foreground">{t('digitalAvatar.states.noAvatars')}</p>
                  <p className="mt-1">{t('digitalAvatar.states.noAvatarsHint')}</p>
                </div>
              ) : (
                avatarSections.map((section) => (
                  <div key={section.key} className="space-y-1.5">
                    <div className="flex items-center justify-between gap-2 px-0.5">
                      <p className="px-0.5 text-[11px] font-medium leading-none text-foreground">{section.title}</p>
                      <span className="inline-flex min-w-5 items-center justify-center rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                        {section.items.length}
                      </span>
                    </div>
                    <div className="space-y-2">
                      {section.items.map(avatar => {
                        const selected = avatar.id === selectedAvatarId;
                        const status = normalizeAvatarStatus(avatar);
                        const projection = avatarProjectionMap[avatar.id];
                        const pendingCount = getAvatarProjectionPendingCount(projection);
                        const activityAt =
                          projection?.portalUpdatedAt || avatar.updatedAt || avatar.createdAt;
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
                            className={`w-full text-left rounded-xl border px-2.5 py-2.5 transition-all ${selected ? 'border-primary/60 bg-primary/8 shadow-sm shadow-primary/5' : 'border-border/60 bg-background/80 hover:border-primary/35 hover:bg-muted/20'}`}
                            onClick={() => setSelectedAvatarId(avatar.id)}
                          >
                            <div className="flex items-start justify-between gap-2">
                              <div className="min-w-0 space-y-1">
                                <div className="flex items-center gap-1.5">
                                  <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${avatarStatusDotClass(status)}`} />
                                  <p className="truncate text-[12px] font-semibold text-foreground">{avatar.name}</p>
                                </div>
                                <p className="truncate font-mono text-[10px] text-muted-foreground">/p/{avatar.slug}</p>
                              </div>
                              <div className="flex shrink-0 items-center gap-1">
                                {pendingCount > 0 && (
                                  <span className="inline-flex items-center rounded-full border border-amber-200 bg-amber-50 px-1.5 py-0.5 text-[10px] text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-300">
                                    {t('digitalAvatar.labels.pendingCount', '待处理')}
                                    {' '}
                                    {pendingCount}
                                  </span>
                                )}
                                <span className={`rounded-full border px-1.5 py-0.5 text-[10px] ${avatarStatusBadgeClass(status)}`}>
                                  {statusLabel}
                                </span>
                              </div>
                            </div>
                            <div
                              className="mt-2 flex items-center gap-2 text-[10px] text-muted-foreground"
                              title={activityAt ? formatDateTime(activityAt) : undefined}
                            >
                              <span>
                                {t('digitalAvatar.list.lastActivity', '最近活动')}
                                {' · '}
                                {activityAt ? formatRelativeTime(activityAt) : t('digitalAvatar.labels.unset')}
                              </span>
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))
              )}
            </CardContent>
          </Card>
          )}

          <div className="relative min-h-0">
          <Card className={`min-h-0 h-full flex flex-col transition-[margin] duration-200 ${!focusMode && inspectorOpen ? 'lg:mr-[372px]' : ''}`}>
            {!focusMode && (
            <CardHeader className={workspaceChromeCollapsed ? 'pb-1 pt-3' : 'pb-2'}>
              <div className={`flex justify-between gap-3 ${workspaceChromeCollapsed ? 'items-center' : 'items-start'}`}>
                <div className="min-w-0">
                  <CardTitle className={workspaceChromeCollapsed ? 'text-[13px] leading-5' : 'text-sm'}>
                    {t('digitalAvatar.workspace.focusTitle', '管理 Agent 对话')}
                  </CardTitle>
                  {!workspaceChromeCollapsed && (
                    <p className="text-caption text-muted-foreground">
                      {selectedAvatar
                        ? t('digitalAvatar.workspace.selectedAvatarConsoleHint', {
                            defaultValue:
                              '当前正在治理「{{name}}」。所有创建、提权、优化与发布都先交给管理 Agent。',
                            name: selectedAvatar.name,
                          })
                        : t('digitalAvatar.workspace.managerConsoleHint')}
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 flex-wrap items-center gap-2">
                  <Button variant="outline" size="sm" className="h-8 px-2.5 text-[11px]" onClick={toggleInspectorPanel}>
                    {inspectorOpen
                      ? t('digitalAvatar.actions.hideConsole', '收起控制台')
                      : `${t('digitalAvatar.actions.showConsole', '打开控制台')} · ${t(`digitalAvatar.inspector.${inspectorTab}` as const, inspectorTab)}`}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 px-2.5 text-[11px]"
                    onClick={() => setWorkspacePanelOpen(true)}
                  >
                    {t('digitalAvatar.workspace.workbenchPanel', {
                      defaultValue: '岗位工作台',
                    })}
                  </Button>
                  <Button variant="outline" size="sm" className="h-8 px-2.5 text-[11px]" onClick={toggleFocusMode}>
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
                      <Button size="sm" variant="outline" onClick={openLaboratory}>
                        <ExternalLink className="w-3.5 h-3.5 mr-1" />
                        {t('digitalAvatar.actions.openLaboratory')}
                      </Button>
                    )}
                  </div>
                </div>
              ) : (
                <div className={`h-full flex flex-col ${focusMode ? 'gap-0' : 'gap-2'}`}>
                  <Dialog open={!focusMode && workspacePanelOpen} onOpenChange={setWorkspacePanelOpen}>
                    <DialogContent className="max-h-[88vh] overflow-hidden p-0 sm:max-w-5xl">
                      <DialogHeader className="border-b border-border/60 px-5 py-4">
                        <DialogTitle>
                          {t('digitalAvatar.workspace.workbenchPanel', {
                            defaultValue: '岗位工作台',
                          })}
                        </DialogTitle>
                        <p className="mt-1 text-sm text-muted-foreground">{workspacePanelSummaryText}</p>
                      </DialogHeader>
                      <div className="max-h-[calc(88vh-88px)] overflow-y-auto px-5 py-4 space-y-3">
                  {!workspaceChromeCollapsed && !selectedAvatar && (
                    <div className="rounded-md border border-primary/20 bg-primary/5 p-2.5 text-caption text-muted-foreground space-y-2">
                      <p className="text-foreground font-medium">
                        {t('digitalAvatar.workspace.bootstrapHintTitle', '先与管理 Agent 对话，再创建分身')}
                      </p>
                      <p>
                        {t(
                          'digitalAvatar.workspace.bootstrapHintBody',
                          '管理 Agent 会先确认目标与能力边界，再调用工具创建并配置数字分身。'
                        )}
                      </p>
                      <div className="flex items-center gap-2">
                        <span className="shrink-0 text-[11px] text-muted-foreground">
                          {t('digitalAvatar.labels.managerAgent')}
                        </span>
                        <select
                          className="h-8 w-full rounded-md border bg-background px-2 text-xs"
                          value={effectiveManagerAgentId}
                          onChange={(e) => setBootstrapManagerAgentId(e.target.value)}
                        >
                          {managerGroupOptions.map((agent) => (
                            <option key={agent.id} value={agent.id}>
                              {getAgentDisplayName(agent, agent.name)}
                            </option>
                          ))}
                        </select>
                      </div>
                    </div>
                  )}
                  {showWorkspaceBootstrapOnly ? (
                    <div className="rounded-lg border border-primary/15 bg-background/95 p-3">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0 flex-1">
                          <p className="text-sm font-medium text-foreground">
                            {t(
                              'digitalAvatar.workspace.bootstrapWorkspaceTitle',
                              { defaultValue: '先让当前管理 Agent 规划一个岗位' },
                            )}
                          </p>
                          <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                            {t(
                              'digitalAvatar.workspace.bootstrapWorkspaceDesc',
                              {
                                defaultValue:
                                  '当前管理组还没有可运营岗位。建议先让管理 Agent 帮你定义服务对象、职责边界和默认工作方式；如需精确控制，再使用顶部的高级创建入口。',
                              },
                            )}
                          </p>
                        </div>
                        <div className="flex shrink-0 flex-wrap gap-2">
                          <Button
                            variant="outline"
                            size="sm"
                            className="h-8 px-2.5 text-[11px]"
                            onClick={handleAskQuestionEntry}
                          >
                            {t('digitalAvatar.workspace.bootstrapWorkspacePlan', {
                              defaultValue: '发起岗位规划',
                            })}
                          </Button>
                        </div>
                      </div>
                      <div className="mt-3 flex flex-wrap items-center gap-2 rounded-lg border border-border/60 bg-muted/10 px-3 py-2">
                        <div className="min-w-0 rounded-full border border-border/60 bg-background/70 px-2.5 py-1 text-[11px] text-muted-foreground">
                          <span className="font-medium text-foreground">
                            {t('digitalAvatar.workspace.bootstrapCurrentManagerTitle', {
                              defaultValue: '当前管理组',
                            })}
                          </span>
                          <span className="mx-1 text-border">·</span>
                          <span className="truncate">
                            {getAgentDisplayName(managerAgent, t('digitalAvatar.states.noManagerAgent'))}
                          </span>
                        </div>
                        <div className="rounded-full border border-border/60 bg-background/70 px-2.5 py-1 text-[11px] text-muted-foreground">
                          <span className="font-medium text-foreground">
                            {t('digitalAvatar.workspace.bootstrapDefaultFlowTitle', {
                              defaultValue: '默认流程',
                            })}
                          </span>
                          <span className="mx-1 text-border">·</span>
                          <span>
                            {t('digitalAvatar.workspace.bootstrapDefaultFlowValue', {
                              defaultValue: '先规划，再创建，再治理',
                            })}
                          </span>
                        </div>
                        <div className="rounded-full border border-border/60 bg-background/70 px-2.5 py-1 text-[11px] text-muted-foreground">
                          <span className="font-medium text-foreground">
                            {t('digitalAvatar.workspace.bootstrapNextStepTitle', {
                              defaultValue: '推荐下一步',
                            })}
                          </span>
                          <span className="mx-1 text-border">·</span>
                          <span>
                            {t('digitalAvatar.workspace.bootstrapNextStepValue', {
                              defaultValue: '先创建一个岗位原型',
                            })}
                          </span>
                        </div>
                      </div>
                    </div>
                  ) : (
                    <div className="space-y-3 rounded-lg border border-border/60 bg-background/95 p-3">
                      {selectedAvatar ? (
                        <div className="flex flex-wrap items-start justify-between gap-3 rounded-lg border border-border/60 bg-muted/10 px-3 py-3">
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-2">
                              <p className="text-sm font-medium text-foreground">{selectedAvatar.name}</p>
                              <AvatarTypeBadge type={selectedAvatarType} />
                              <span className={`rounded-full border px-2 py-1 text-[10px] ${avatarStatusBadgeClass(normalizeAvatarStatus(selectedAvatar))}`}>
                                {selectedAvatarStatusLabel}
                              </span>
                            </div>
                            <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                              {selectedAvatarSummaryText}
                            </p>
                          </div>
                          {hasWorkbenchContent ? (
                            <Button
                              variant="ghost"
                              size="sm"
                              className="h-8 px-3 text-[11px]"
                              onClick={() => navigate(`/teams/${teamId}/digital-avatars/${selectedAvatar.id}/timeline`)}
                            >
                              {t('digitalAvatar.workspace.openTimeline', '查看时间线')}
                            </Button>
                          ) : null}
                        </div>
                      ) : (
                        <>
                          <div className="flex flex-wrap items-start justify-between gap-3">
                            <div className="min-w-0 flex-1">
                              <div className="flex flex-wrap items-center gap-2">
                                <p className="text-sm font-medium text-foreground">
                                  {t('digitalAvatar.workspace.managerGroupOverviewTitle', { defaultValue: '管理组概览' })}
                                </p>
                              </div>
                              <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                {managerScopeSummaryText}
                              </p>
                            </div>
                            <Button size="sm" className="h-8 px-2.5 text-[11px]" onClick={handleRoleOverviewPrimaryAction}>
                              {roleOverviewPrimaryAction.label}
                            </Button>
                          </div>
                          <div className="flex flex-wrap gap-1.5">
                            <SummaryPill label={t('digitalAvatar.list.title', '分身列表')} value={String(managerGroupStats.total)} accent />
                            <SummaryPill label={t('digitalAvatar.filters.external')} value={String(managerGroupStats.external)} />
                            <SummaryPill label={t('digitalAvatar.filters.internal')} value={String(managerGroupStats.internal)} />
                            <SummaryPill label={t('digitalAvatar.status.published', '已发布')} value={String(managerGroupStats.published)} />
                            <SummaryPill
                              label={t('digitalAvatar.workspace.summaryStatus', '待处理')}
                              value={String(managerGroupStats.pending)}
                              accent={managerGroupStats.pending > 0}
                            />
                          </div>

                          {managerPreviewAvatars.length > 0 ? (
                            <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                              <div className="flex flex-wrap items-center justify-between gap-2">
                                <div>
                                  <p className="text-xs font-medium text-foreground">
                                    {t('digitalAvatar.workspace.groupRolesTitle', { defaultValue: '当前管理组岗位' })}
                                  </p>
                                  <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                    {t('digitalAvatar.workspace.groupRolesHint', {
                                      defaultValue: '先选择一个岗位，再进入问答、共同处理或长程委托。',
                                    })}
                                  </p>
                                </div>
                                {managerGroupStats.total > managerPreviewAvatars.length ? (
                                  <span className="rounded-full border border-border/60 bg-background px-2 py-1 text-[10px] text-muted-foreground">
                                    +{managerGroupStats.total - managerPreviewAvatars.length}
                                  </span>
                                ) : null}
                              </div>
                              <div className="mt-2 grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                                {managerPreviewAvatars.map((avatar) => {
                                  const projection = avatarProjectionMap[avatar.id];
                                  const avatarType = detectAvatarType(avatar, projection);
                                  const avatarStatus = normalizeAvatarStatus(avatar);
                                  return (
                                    <button
                                      key={avatar.id}
                                      type="button"
                                      className="rounded-lg border border-border/60 bg-background px-3 py-2 text-left hover:border-primary/30"
                                      onClick={() => setSelectedAvatarId(avatar.id)}
                                    >
                                      <div className="flex flex-wrap items-start justify-between gap-2">
                                        <div className="min-w-0">
                                          <p className="truncate text-[11px] font-medium text-foreground">{avatar.name}</p>
                                          <p className="mt-0.5 truncate text-[10px] text-muted-foreground">
                                            {avatar.slug ? `/p/${avatar.slug}` : avatar.id}
                                          </p>
                                        </div>
                                        <div className="flex items-center gap-1.5">
                                          <AvatarTypeBadge type={avatarType} className="h-5 px-1.5 text-[10px]" />
                                          <span className={`rounded-full border px-1.5 py-0.5 text-[10px] ${avatarStatusBadgeClass(avatarStatus)}`}>
                                            {avatarStatusLabel(avatarStatus, t)}
                                          </span>
                                        </div>
                                      </div>
                                    </button>
                                  );
                                })}
                              </div>
                            </div>
                          ) : (
                            <div className="rounded-lg border border-dashed border-border/60 bg-muted/5 p-3">
                              <p className="text-xs font-medium text-foreground">
                                {t('digitalAvatar.workspace.groupRolesEmptyTitle', {
                                  defaultValue: '当前管理组还没有岗位',
                                })}
                              </p>
                              <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                {t('digitalAvatar.workspace.groupRolesEmpty', {
                                  defaultValue: '先让管理 Agent 规划一个岗位，必要时再通过顶部高级入口精确创建。',
                                })}
                              </p>
                            </div>
                          )}
                        </>
                      )}
                    </div>
                  )}
                  {effectiveManagerAgentId && !showWorkspaceBootstrapOnly && (
                    <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <p className="text-sm font-medium text-foreground">
                          {t('digitalAvatar.workspace.workEntryTitle', '把工作交给当前数字岗位')}
                        </p>
                        <div className="flex flex-wrap gap-1.5">
                          <Button
                            variant={workEntryMode === 'ask' ? 'default' : 'outline'}
                            size="sm"
                            className="h-8 px-2.5 text-[11px]"
                            onClick={() => setWorkEntryMode('ask')}
                          >
                            {t('digitalAvatar.workspace.entryAsk', '问问题')}
                          </Button>
                          <Button
                            variant={workEntryMode === 'task' ? 'default' : 'outline'}
                            size="sm"
                            className="h-8 px-2.5 text-[11px]"
                            onClick={() => setWorkEntryMode('task')}
                          >
                            {t('digitalAvatar.workspace.entryTask', '交任务')}
                          </Button>
                          <Button
                            variant={workEntryMode === 'collaborate' ? 'default' : 'outline'}
                            size="sm"
                            className="h-8 px-2.5 text-[11px]"
                            onClick={() => setWorkEntryMode('collaborate')}
                          >
                            {t('digitalAvatar.workspace.entryCollaborate', '共同处理')}
                          </Button>
                          <Button
                            variant={workEntryMode === 'mission' ? 'default' : 'outline'}
                            size="sm"
                            className="h-8 px-2.5 text-[11px]"
                            onClick={() => setWorkEntryMode('mission')}
                          >
                            {t('digitalAvatar.workspace.entryMission', '长程委托')}
                          </Button>
                        </div>
                      </div>

                      <div className="mt-3 rounded-md border border-border/60 bg-background p-3">
                        {workEntryMode === 'task' ? (
                          <div className="space-y-3">
                            <div>
                              <p className="text-xs font-medium text-foreground">
                                {t('digitalAvatar.workspace.taskGoalLabel', '任务目标')}
                              </p>
                              <textarea
                                className="mt-1 min-h-[76px] w-full rounded-md border bg-background px-3 py-2 text-sm"
                                value={taskGoalDraft}
                                onChange={(event) => setTaskGoalDraft(event.target.value)}
                                placeholder={t(
                                  'digitalAvatar.workspace.taskGoalPlaceholder',
                                  '例如：整理一份对外可用的服务说明，并给出需要人工确认的项。'
                                )}
                              />
                            </div>
                            <div>
                              <p className="text-xs font-medium text-foreground">
                                {t('digitalAvatar.workspace.taskOutcomeLabel', '期望交付')}
                              </p>
                              <input
                                className="mt-1 h-9 w-full rounded-md border bg-background px-3 text-sm"
                                value={taskOutcomeDraft}
                                onChange={(event) => setTaskOutcomeDraft(event.target.value)}
                                placeholder={t(
                                  'digitalAvatar.workspace.taskOutcomePlaceholder',
                                  '例如：一页说明稿、一个草稿版本、三条执行建议'
                                )}
                              />
                            </div>
                            <div className="flex justify-end">
                              <Button size="sm" onClick={handlePrepareTaskDelegation}>
                                {t('digitalAvatar.workspace.prepareTaskPrompt', '填入任务委托')}
                              </Button>
                            </div>
                          </div>
                        ) : workEntryMode === 'collaborate' ? (
                          <div className="space-y-3">
                            <div className="flex flex-wrap items-start justify-between gap-3">
                              <div>
                                <p className="text-xs font-medium text-foreground">
                                  {t('digitalAvatar.workspace.collaboratePanelTitle', '共同处理当前工作对象')}
                                </p>
                                <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                  {selectedAvatarBoundDocCount > 0
                                    ? t(
                                        'digitalAvatar.workspace.collaboratePanelBoundHint',
                                        '当前岗位已绑定 {{count}} 个工作对象。你可以先查看对象内容，再把改写、审阅或共创要求交给管理 Agent。',
                                        { count: selectedAvatarBoundDocCount }
                                      )
                                    : t(
                                        'digitalAvatar.workspace.collaboratePanelNoDocHint',
                                        '当前岗位还没有绑定工作对象。先让管理 Agent 明确对象范围，或先去文件区准备材料。'
                                      )}
                                </p>
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button variant="outline" size="sm" onClick={handleOpenDocumentsWorkspace}>
                                  {t('digitalAvatar.workspace.openDocuments', '打开文件区')}
                                </Button>
                                <Button size="sm" onClick={handlePrepareCollaborate}>
                                  {t('digitalAvatar.workspace.prepareCollaboratePrompt', '填入共同处理请求')}
                                </Button>
                              </div>
                            </div>
                            {preferredWorkspaceObjectSummary && (
                              <div className="rounded-md border border-primary/15 bg-primary/5 px-3 py-2">
                                <div className="flex flex-wrap items-center gap-2">
                                  <span className="text-[11px] font-medium text-foreground">
                                    {t('digitalAvatar.workspace.currentObjectTitle', { defaultValue: '当前对象' })}
                                  </span>
                                  <span className="text-[11px] text-foreground">
                                    {preferredWorkspaceObjectSummary.doc.display_name || preferredWorkspaceObjectSummary.doc.name}
                                  </span>
                                  <span className={`rounded-full border px-1.5 py-0.5 text-[10px] ${workspaceObjectKindBadgeClass(preferredWorkspaceObjectSummary.kind)}`}>
                                    {preferredWorkspaceObjectSummary.kindLabel}
                                  </span>
                                </div>
                                <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                  {t(
                                    'digitalAvatar.workspace.currentObjectHint',
                                    {
                                      defaultValue:
                                        '先围绕当前对象决定是直接审阅、生成任务，还是在预览与版本视图里继续协作。',
                                    },
                                  )}
                                </p>
                              </div>
                            )}
                            <div className="rounded-md border border-border/60 bg-background px-3 py-2">
                              <div className="flex flex-wrap items-center justify-between gap-2">
                                <div>
                                  <p className="text-xs font-medium text-foreground">
                                    {workspaceSurfaceTitle(preferredWorkspaceObjectSummary?.kind, t)}
                                  </p>
                                  <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                    {workspaceSurfaceDescription(preferredWorkspaceObjectSummary?.kind, t)}
                                  </p>
                                </div>
                                {preferredWorkspaceObjectSummary ? (
                                  <span className={`rounded-full border px-2 py-1 text-[10px] ${workspaceObjectKindBadgeClass(preferredWorkspaceObjectSummary.kind)}`}>
                                    {preferredWorkspaceObjectSummary.kindLabel}
                                  </span>
                                ) : (
                                  <span className="rounded-full border border-border/60 bg-muted/20 px-2 py-1 text-[10px] text-muted-foreground">
                                    {t('digitalAvatar.workspace.objectKindOther', '对象')}
                                  </span>
                                )}
                              </div>
                            </div>
                            {workspaceRecommendations.length > 0 && (
                              <div className="grid gap-2 lg:grid-cols-2">
                                {workspaceRecommendations.map((recommendation) => (
                                  <button
                                    key={recommendation.key}
                                    type="button"
                                    className="flex items-start justify-between gap-3 rounded-md border border-border/60 bg-background px-3 py-2 text-left hover:border-primary/30 hover:bg-muted/15"
                                    onClick={() => {
                                      void handleApplyWorkspaceRecommendation(recommendation);
                                    }}
                                  >
                                    <div className="min-w-0">
                                      <p className="text-[11px] font-medium text-foreground">{recommendation.label}</p>
                                      <p className="mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
                                        {recommendation.description}
                                      </p>
                                    </div>
                                    <span className="shrink-0 rounded-full border border-primary/20 bg-primary/5 px-2 py-1 text-[10px] text-primary">
                                      {recommendation.mode === 'task'
                                        ? t('digitalAvatar.workspace.entryTask', '交任务')
                                        : recommendation.mode === 'ask'
                                        ? t('digitalAvatar.workspace.entryAsk', '问问题')
                                        : t('digitalAvatar.workspace.entryCollaborate', '共同处理')}
                                    </span>
                                  </button>
                                ))}
                              </div>
                            )}
                            {selectedAvatarBoundDocCount > 0 ? (
                              <div className="grid gap-3 xl:grid-cols-[220px_minmax(0,1fr)]">
                                <div className="rounded-md border border-border/60 bg-muted/10 p-2">
                                  <div className="mb-2 flex items-center justify-between gap-2">
                                    <div>
                                      <p className="text-xs font-medium text-foreground">
                                        {t('digitalAvatar.workspace.boundDocsTitle', '当前工作对象')}
                                      </p>
                                      <p className="text-[10px] text-muted-foreground">
                                        {workspaceDocumentsLoading
                                          ? t('digitalAvatar.workspace.loadingBoundDocs', '正在加载对象...')
                                          : t('digitalAvatar.workspace.boundDocsCount', '共 {{count}} 个', {
                                              count: workspaceDocuments.length,
                                            })}
                                      </p>
                                    </div>
                                  </div>
                                  {workspaceObjectSummaryPills.length > 0 ? (
                                    <div className="mb-2 flex flex-wrap gap-1.5">
                                      {workspaceObjectSummaryPills.map((item) => (
                                        <span
                                          key={item.key}
                                          className={`inline-flex items-center gap-1 rounded-full border px-2 py-1 text-[10px] ${workspaceObjectKindBadgeClass(item.key)}`}
                                        >
                                          <span>{item.label}</span>
                                          <span className="font-medium">{item.count}</span>
                                        </span>
                                      ))}
                                    </div>
                                  ) : null}
                                  <div className="space-y-1.5">
                                    {workspaceDocuments.length === 0 ? (
                                      <div className="rounded-md border border-dashed border-border/60 bg-background/70 px-3 py-2 text-[11px] text-muted-foreground">
                                        {workspaceDocumentsLoading
                                          ? t('common.loading', '加载中...')
                                          : t('digitalAvatar.workspace.boundDocsEmpty', '暂未加载到工作对象详情，请前往文件区查看。')}
                                      </div>
                                    ) : workspaceObjectSummaries.map((item) => {
                                      const { doc, kind, kindLabel } = item;
                                      const active = selectedWorkspaceDocumentId === doc.id;
                                      return (
                                        <button
                                          key={doc.id}
                                          type="button"
                                          className={`w-full rounded-md border px-3 py-2 text-left transition-colors ${
                                            active
                                              ? 'border-primary/40 bg-primary/5'
                                              : 'border-border/60 bg-background hover:border-primary/20 hover:bg-muted/20'
                                          }`}
                                          onClick={() => {
                                            void handleSelectWorkspaceDocument(doc.id);
                                          }}
                                        >
                                          <div className="flex items-start justify-between gap-2">
                                            <p className="min-w-0 truncate text-[11px] font-medium text-foreground">
                                              {doc.display_name || doc.name}
                                            </p>
                                            <span className={`shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] ${workspaceObjectKindBadgeClass(kind)}`}>
                                              {kindLabel}
                                            </span>
                                          </div>
                                          <p className="mt-1 truncate text-[10px] text-muted-foreground">
                                            {doc.folder_path || doc.mime_type}
                                          </p>
                                        </button>
                                      );
                                    })}
                                  </div>
                                </div>
                                <div className="overflow-hidden rounded-md border border-border/60 bg-background/80">
                                  {selectedWorkspaceDocument ? (
                                    <div className="flex h-[420px] flex-col">
                                      <div className="flex items-center justify-between gap-2 border-b border-border/60 bg-muted/10 px-3 py-2">
                                        <div>
                                          <p className="text-xs font-medium text-foreground">
                                            {selectedWorkspaceDocument.display_name || selectedWorkspaceDocument.name}
                                          </p>
                                          <p className="text-[10px] text-muted-foreground">
                                            {selectedWorkspaceObjectSummary
                                              ? `${selectedWorkspaceObjectSummary.kindLabel} · ${selectedWorkspaceDocument.mime_type}`
                                              : selectedWorkspaceDocument.mime_type}
                                          </p>
                                        </div>
                                        <div className="flex flex-wrap gap-1.5">
                                          <Button
                                            variant={workspaceDocumentMode === 'guide' ? 'default' : 'outline'}
                                            size="sm"
                                            className="h-7 px-2 text-[11px]"
                                            onClick={() => setWorkspaceDocumentMode('guide')}
                                          >
                                            {t('digitalAvatar.workspace.docModeGuide', '处理方式')}
                                          </Button>
                                          <Button
                                            variant={workspaceDocumentMode === 'preview' ? 'default' : 'outline'}
                                            size="sm"
                                            className="h-7 px-2 text-[11px]"
                                            onClick={() => setWorkspaceDocumentMode('preview')}
                                          >
                                            {workspacePreviewModeLabel(selectedWorkspaceObjectSummary?.kind, t)}
                                          </Button>
                                          {workspaceDocumentEditable && (
                                            <Button
                                              variant={workspaceDocumentMode === 'edit' ? 'default' : 'outline'}
                                              size="sm"
                                              className="h-7 px-2 text-[11px]"
                                              onClick={handleWorkspaceDocumentEdit}
                                              disabled={workspaceDocumentLoading}
                                            >
                                              {workspaceDocumentLoading
                                                ? t('common.loading', '加载中...')
                                                : t('digitalAvatar.workspace.docModeEdit', '编辑')}
                                            </Button>
                                          )}
                                          <Button
                                            variant={workspaceDocumentMode === 'versions' || workspaceDocumentMode === 'diff' ? 'default' : 'outline'}
                                            size="sm"
                                            className="h-7 px-2 text-[11px]"
                                            onClick={handleWorkspaceDocumentVersions}
                                          >
                                            {workspaceHistoryModeLabel(selectedWorkspaceObjectSummary?.kind, t)}
                                          </Button>
                                        </div>
                                      </div>
                                      <div className="min-h-0 flex-1">
                                        {workspaceDocumentMode === 'guide' ? (
                                          <div className="grid h-full gap-3 overflow-auto p-3 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,0.9fr)]">
                                            <div className="space-y-3">
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {workspaceSurfaceGuideTitle(selectedWorkspaceObjectSummary?.kind, t)}
                                                </p>
                                                <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                                  {workspaceSurfaceDescription(selectedWorkspaceObjectSummary?.kind, t)}
                                                </p>
                                                <div className="mt-3 space-y-2">
                                                  {workspaceSurfaceFocusItems(selectedWorkspaceObjectSummary?.kind, t).map((item) => (
                                                    <div
                                                      key={item}
                                                      className="rounded-md border border-border/60 bg-background px-3 py-2 text-[11px] leading-5 text-muted-foreground"
                                                    >
                                                      {item}
                                                    </div>
                                                  ))}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {t('digitalAvatar.workspace.surfaceDeliverablesTitle', '推荐交付')}
                                                </p>
                                                <div className="mt-2 flex flex-wrap gap-1.5">
                                                  {workspaceSurfaceDeliverables(selectedWorkspaceObjectSummary?.kind, t).map((item) => (
                                                    <span
                                                      key={item}
                                                      className="inline-flex rounded-full border border-border/60 bg-background px-2 py-1 text-[10px] text-foreground"
                                                    >
                                                      {item}
                                                    </span>
                                                  ))}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {workspaceSurfaceWorkflowTitle(selectedWorkspaceObjectSummary?.kind, t)}
                                                </p>
                                                <div className="mt-2 space-y-2">
                                                  {workspaceSurfaceWorkflowSteps(selectedWorkspaceObjectSummary?.kind, t).map((item) => (
                                                    <div
                                                      key={item}
                                                      className="rounded-md border border-border/60 bg-background px-3 py-2 text-[11px] leading-5 text-muted-foreground"
                                                    >
                                                      {item}
                                                    </div>
                                                  ))}
                                                </div>
                                              </div>
                                            </div>
                                            <div className="space-y-3">
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {t('digitalAvatar.workspace.surfaceRecommendationTitle', '对象推荐动作')}
                                                </p>
                                                <div className="mt-2 space-y-2">
                                                  {workspaceRecommendations.length > 0 ? (
                                                    workspaceRecommendations.map((recommendation) => (
                                                      <button
                                                        key={recommendation.key}
                                                        type="button"
                                                        className="flex w-full items-start justify-between gap-2 rounded-md border border-border/60 bg-background px-3 py-2 text-left hover:border-primary/30"
                                                        onClick={() => {
                                                          void handleApplyWorkspaceRecommendation(recommendation);
                                                        }}
                                                      >
                                                        <div className="min-w-0">
                                                          <p className="text-[11px] font-medium text-foreground">{recommendation.label}</p>
                                                          <p className="mt-0.5 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
                                                            {recommendation.description}
                                                          </p>
                                                        </div>
                                                        <span className="shrink-0 rounded-full border border-primary/20 bg-primary/5 px-2 py-1 text-[10px] text-primary">
                                                          {recommendation.mode === 'task'
                                                            ? t('digitalAvatar.workspace.entryTask', '交任务')
                                                            : recommendation.mode === 'ask'
                                                            ? t('digitalAvatar.workspace.entryAsk', '问问题')
                                                            : t('digitalAvatar.workspace.entryCollaborate', '共同处理')}
                                                        </span>
                                                      </button>
                                                    ))
                                                  ) : (
                                                    <div className="rounded-md border border-dashed border-border/60 bg-background px-3 py-2 text-[11px] text-muted-foreground">
                                                      {t('digitalAvatar.workspace.surfaceRecommendationEmpty', '当前对象还没有推荐动作，可以直接提问或发起长程委托。')}
                                                    </div>
                                                  )}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {t('digitalAvatar.workspace.surfaceBoundaryTitle', '边界提醒')}
                                                </p>
                                                <div className="mt-2 space-y-2">
                                                  {workspaceSurfaceBoundaryItems(selectedWorkspaceObjectSummary?.kind, t).map((item) => (
                                                    <div
                                                      key={item}
                                                      className="rounded-md border border-border/60 bg-background px-3 py-2 text-[11px] leading-5 text-muted-foreground"
                                                    >
                                                      {item}
                                                    </div>
                                                  ))}
                                                </div>
                                              </div>
                                            </div>
                                          </div>
                                        ) : workspaceDocumentMode === 'edit' && workspaceDocumentLock ? (
                                          <DocumentEditor
                                            teamId={teamId}
                                            document={selectedWorkspaceDocument}
                                            initialContent={workspaceDocumentText}
                                            lock={workspaceDocumentLock}
                                            onSave={handleWorkspaceDocumentEditSaved}
                                            onClose={handleWorkspaceDocumentEditClosed}
                                          />
                                        ) : workspaceDocumentMode === 'versions' ? (
                                          <VersionTimeline
                                            teamId={teamId}
                                            docId={selectedWorkspaceDocument.id}
                                            canManage={false}
                                            onCompare={handleWorkspaceDocumentCompare}
                                          />
                                        ) : workspaceDocumentMode === 'diff' && workspaceDocumentCompareVersions ? (
                                          <VersionDiff
                                            teamId={teamId}
                                            docId={selectedWorkspaceDocument.id}
                                            version1={workspaceDocumentCompareVersions[0]}
                                            version2={workspaceDocumentCompareVersions[1]}
                                            onClose={() => setWorkspaceDocumentMode('versions')}
                                          />
                                        ) : selectedWorkspaceObjectSummary?.kind
                                          && selectedWorkspaceObjectSummary.kind !== 'document' ? (
                                          <div className="grid h-full gap-3 overflow-auto p-3 lg:grid-cols-[minmax(0,1.45fr)_minmax(280px,0.85fr)]">
                                            <div className="min-h-0 overflow-hidden rounded-lg border border-border/60 bg-background">
                                              <Suspense fallback={<DocumentPreviewLoading />}>
                                                <DocumentPreview
                                                  teamId={teamId}
                                                  document={selectedWorkspaceDocument}
                                                  onClose={() => {
                                                    void handleSelectWorkspaceDocument(null);
                                                  }}
                                                  onEdit={workspaceDocumentEditable ? handleWorkspaceDocumentEdit : undefined}
                                                  onVersions={handleWorkspaceDocumentVersions}
                                                />
                                              </Suspense>
                                            </div>
                                            <div className="space-y-3">
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {workspacePreviewSurfaceTitle(selectedWorkspaceObjectSummary.kind, t)}
                                                </p>
                                                <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                                  {workspacePreviewSurfaceHint(selectedWorkspaceObjectSummary.kind, t)}
                                                </p>
                                                <div className="mt-3 space-y-2">
                                                  {workspacePreviewSurfaceHighlights(selectedWorkspaceObjectSummary.kind, t).map((item) => (
                                                    <div
                                                      key={item}
                                                      className="rounded-md border border-border/60 bg-background px-3 py-2 text-[11px] leading-5 text-muted-foreground"
                                                    >
                                                      {item}
                                                    </div>
                                                  ))}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <div className="flex items-center justify-between gap-2">
                                                  <p className="text-xs font-medium text-foreground">
                                                    {t('digitalAvatar.workspace.surfaceDeliverablesTitle', '推荐交付')}
                                                  </p>
                                                  <span className={`rounded-full border px-2 py-1 text-[10px] ${workspaceObjectKindBadgeClass(selectedWorkspaceObjectSummary.kind)}`}>
                                                    {selectedWorkspaceObjectSummary.kindLabel}
                                                  </span>
                                                </div>
                                                <div className="mt-2 flex flex-wrap gap-1.5">
                                                  {workspaceSurfaceDeliverables(selectedWorkspaceObjectSummary.kind, t).map((item) => (
                                                    <span
                                                      key={item}
                                                      className="inline-flex rounded-full border border-border/60 bg-background px-2 py-1 text-[10px] text-foreground"
                                                    >
                                                      {item}
                                                    </span>
                                                  ))}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {t('digitalAvatar.workspace.surfaceRecommendationTitle', '对象推荐动作')}
                                                </p>
                                                <div className="mt-2 space-y-2">
                                                  {workspaceRecommendations.length > 0 ? (
                                                    workspaceRecommendations.slice(0, 3).map((recommendation) => (
                                                      <button
                                                        key={recommendation.key}
                                                        type="button"
                                                        className="flex w-full items-start justify-between gap-2 rounded-md border border-border/60 bg-background px-3 py-2 text-left hover:border-primary/30"
                                                        onClick={() => {
                                                          void handleApplyWorkspaceRecommendation(recommendation);
                                                        }}
                                                      >
                                                        <div className="min-w-0">
                                                          <p className="text-[11px] font-medium text-foreground">{recommendation.label}</p>
                                                          <p className="mt-0.5 line-clamp-2 text-[10px] leading-4 text-muted-foreground">
                                                            {recommendation.description}
                                                          </p>
                                                        </div>
                                                        <span className="shrink-0 rounded-full border border-primary/20 bg-primary/5 px-2 py-1 text-[10px] text-primary">
                                                          {recommendation.mode === 'task'
                                                            ? t('digitalAvatar.workspace.entryTask', '交任务')
                                                            : recommendation.mode === 'ask'
                                                            ? t('digitalAvatar.workspace.entryAsk', '问问题')
                                                            : t('digitalAvatar.workspace.entryCollaborate', '共同处理')}
                                                        </span>
                                                      </button>
                                                    ))
                                                  ) : (
                                                    <div className="rounded-md border border-dashed border-border/60 bg-background px-3 py-2 text-[11px] text-muted-foreground">
                                                      {t('digitalAvatar.workspace.surfaceRecommendationEmpty', '当前对象还没有推荐动作，可以直接提问或发起长程委托。')}
                                                    </div>
                                                  )}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {workspaceSurfaceWorkflowTitle(selectedWorkspaceObjectSummary.kind, t)}
                                                </p>
                                                <div className="mt-2 space-y-2">
                                                  {workspaceSurfaceWorkflowSteps(selectedWorkspaceObjectSummary.kind, t).map((item) => (
                                                    <div
                                                      key={item}
                                                      className="rounded-md border border-border/60 bg-background px-3 py-2 text-[11px] leading-5 text-muted-foreground"
                                                    >
                                                      {item}
                                                    </div>
                                                  ))}
                                                </div>
                                              </div>
                                              <div className="rounded-lg border border-border/60 bg-muted/10 p-3">
                                                <p className="text-xs font-medium text-foreground">
                                                  {t('digitalAvatar.workspace.surfaceBoundaryTitle', '边界提醒')}
                                                </p>
                                                <div className="mt-2 space-y-2">
                                                  {workspaceSurfaceBoundaryItems(selectedWorkspaceObjectSummary.kind, t).map((item) => (
                                                    <div
                                                      key={item}
                                                      className="rounded-md border border-border/60 bg-background px-3 py-2 text-[11px] leading-5 text-muted-foreground"
                                                    >
                                                      {item}
                                                    </div>
                                                  ))}
                                                </div>
                                              </div>
                                            </div>
                                          </div>
                                        ) : (
                                          <Suspense fallback={<DocumentPreviewLoading />}>
                                            <DocumentPreview
                                              teamId={teamId}
                                              document={selectedWorkspaceDocument}
                                              onClose={() => {
                                                void handleSelectWorkspaceDocument(null);
                                              }}
                                              onEdit={workspaceDocumentEditable ? handleWorkspaceDocumentEdit : undefined}
                                              onVersions={handleWorkspaceDocumentVersions}
                                            />
                                          </Suspense>
                                        )}
                                      </div>
                                    </div>
                                  ) : (
                                    <div className="flex h-[420px] items-center justify-center px-6 text-center text-sm text-muted-foreground">
                                      {t('digitalAvatar.workspace.selectDocToPreview', '选择一个工作对象后，可在这里预览、编辑或审阅差异，再交给管理 Agent 继续协同。')}
                                    </div>
                                  )}
                                </div>
                              </div>
                            ) : null}
                            <div className="grid gap-2 sm:grid-cols-3">
                              {[
                                t('digitalAvatar.workspace.collaborateExample1', '审阅一份协议、指南或页面内容，并标出需要调整的重点段落'),
                                t('digitalAvatar.workspace.collaborateExample2', '围绕当前对象生成草稿修改版本，并用版本 / Diff 解释变化'),
                                t('digitalAvatar.workspace.collaborateExample3', '结合绑定对象回答问题，再给出下一步修改、发布或委托建议'),
                              ].map((item) => (
                                <div key={item} className="rounded-md border border-border/60 bg-muted/15 px-3 py-2 text-[11px] text-muted-foreground">
                                  {item}
                                </div>
                              ))}
                            </div>
                          </div>
                        ) : workEntryMode === 'mission' ? (
                          <div className="space-y-3">
                            <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                              <div>
                                <p className="text-xs font-medium text-foreground">
                                  {t('digitalAvatar.workspace.missionGoalLabel', '长程委托目标')}
                                </p>
                                <textarea
                                  className="mt-1 min-h-[88px] w-full rounded-md border bg-background px-3 py-2 text-sm"
                                  value={missionGoalDraft}
                                  onChange={(event) => setMissionGoalDraft(event.target.value)}
                                  placeholder={t(
                                    'digitalAvatar.workspace.missionGoalPlaceholder',
                                    '例如：基于当前岗位与绑定文档，完成一套对外服务资料，并输出可审阅交付物。'
                                  )}
                                />
                              </div>
                              <div>
                                <p className="text-xs font-medium text-foreground">
                                  {t('digitalAvatar.workspace.missionContextLabel', '补充说明（可选）')}
                                </p>
                                <textarea
                                  className="mt-1 min-h-[88px] w-full rounded-md border bg-background px-3 py-2 text-sm"
                                  value={missionContextDraft}
                                  onChange={(event) => setMissionContextDraft(event.target.value)}
                                  placeholder={t(
                                    'digitalAvatar.workspace.missionContextPlaceholder',
                                    '可以补充交付格式、时间要求、注意事项、审批边界。'
                                  )}
                                />
                              </div>
                            </div>
                            <div className="flex flex-wrap items-center justify-between gap-3">
                              <p className="text-[11px] text-muted-foreground">
                                {t(
                                  'digitalAvatar.workspace.missionBoundDocHint',
                                  '将由管理 Agent 发起长程委托，并自动带上当前绑定文档、岗位类型与权限边界。'
                                )}
                              </p>
                              <Button size="sm" onClick={handleLaunchMission} disabled={creatingMission}>
                                {creatingMission
                                  ? t('digitalAvatar.workspace.missionStarting', '启动中...')
                                  : t('digitalAvatar.workspace.startMission', '发起长程委托')}
                              </Button>
                            </div>
                          </div>
                        ) : (
                          <div className="flex flex-wrap items-start justify-between gap-3">
                            <div>
                              <p className="text-xs font-medium text-foreground">
                                {t('digitalAvatar.workspace.askPanelTitle', '先用管理 Agent 判断问题类型')}
                              </p>
                              <p className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                {t(
                                  'digitalAvatar.workspace.askPanelHint',
                                  '如果只是咨询或解释，直接在输入框提问；如果需要共同处理或长程执行，管理 Agent 会给出下一步建议。'
                                )}
                              </p>
                            </div>
                            <Button size="sm" onClick={handleAskQuestionEntry}>
                              {t('digitalAvatar.workspace.fillAskPrompt', '填入提问引导')}
                            </Button>
                          </div>
                        )}
                      </div>
                    </div>
                  )}
                  {(workspaceMissionsLoading || workspaceMissions.length > 0) && (
                    <div className="mb-3 rounded-lg border border-border/60 bg-muted/10 p-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div>
                          <p className="text-sm font-medium text-foreground">
                            {t('digitalAvatar.workspace.missionPanelTitle', '最近长程委托')}
                          </p>
                          <p className="text-[11px] text-muted-foreground">
                            {t(
                              'digitalAvatar.workspace.missionPanelHint',
                              '复杂工作会在这里沉淀为可跟踪任务。你可以继续查看进度、恢复执行或直接打开详情。'
                            )}
                          </p>
                        </div>
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-8 px-2.5 text-[11px]"
                          onClick={() => setWorkEntryMode('mission')}
                        >
                          {t('digitalAvatar.workspace.entryMission', '长程委托')}
                        </Button>
                      </div>
                      {featuredWorkspaceMission && (
                        <div className="mt-3 rounded-lg border border-primary/15 bg-background/90 p-3">
                          <div className="flex flex-wrap items-start justify-between gap-3">
                            <div className="min-w-0 flex-1">
                              <div className="flex flex-wrap items-center gap-2">
                                <p className="truncate text-sm font-medium text-foreground">
                                  {featuredWorkspaceMission.goal}
                                </p>
                                <span className={`rounded-full border px-2 py-1 text-[10px] ${missionStatusBadgeClass(featuredWorkspaceMission.status)}`}>
                                  {t(`missions.status.${featuredWorkspaceMission.status}` as const, featuredWorkspaceMission.status)}
                                </span>
                              </div>
                              <p className="mt-1 text-[11px] text-muted-foreground">
                                {featuredWorkspaceMissionDetail?.final_summary
                                  || t(
                                    'digitalAvatar.workspace.missionDeliveryFallback',
                                    '任务已进入可跟踪执行，完成后会在这里汇总交付摘要与产出物。'
                                  )}
                              </p>
                              <p className="mt-2 text-[10px] text-muted-foreground">
                                {t('digitalAvatar.workspace.missionProgress', '步骤 {{done}} / {{total}}', {
                                  done: featuredWorkspaceMission.completed_steps,
                                  total: featuredWorkspaceMission.step_count || 0,
                                })}
                                {' · '}
                                {formatRelativeTime(featuredWorkspaceMission.updated_at)}
                              </p>
                            </div>
                            <div className="flex flex-wrap gap-2">
                              <Button
                                variant="outline"
                                size="sm"
                                className="h-8 px-2.5 text-[11px]"
                                onClick={() => handleOpenMissionDetail(featuredWorkspaceMission.mission_id)}
                              >
                                {t('digitalAvatar.workspace.openMissionDetail', '查看进度')}
                              </Button>
                              {(featuredWorkspaceMission.status === 'failed' || featuredWorkspaceMission.status === 'paused') && (
                                <Button
                                  size="sm"
                                  className="h-8 px-2.5 text-[11px]"
                                  onClick={() => handleResumeWorkspaceMission(featuredWorkspaceMission.mission_id)}
                                >
                                  {t('digitalAvatar.workspace.resumeMission', '继续处理')}
                                </Button>
                              )}
                            </div>
                          </div>
                          <div className="mt-3 rounded-md border border-border/60 bg-muted/10 px-3 py-2">
                            <div className="flex items-center justify-between gap-2">
                              <p className="text-xs font-medium text-foreground">
                                {t('digitalAvatar.workspace.missionDeliverablesTitle', '当前交付物')}
                              </p>
                              {workspaceMissionMetaLoading && (
                                <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                              )}
                            </div>
                            {featuredWorkspaceMissionArtifacts.length > 0 ? (
                              <div className="mt-2 flex flex-wrap gap-2">
                                {featuredWorkspaceMissionArtifacts.slice(0, 4).map((artifact) => (
                                  <a
                                    key={artifact.artifact_id}
                                    href={missionApi.getArtifactDownloadUrl(artifact.artifact_id)}
                                    className="inline-flex max-w-full items-center gap-1 rounded-full border border-border/60 bg-background px-2.5 py-1 text-[11px] text-foreground hover:border-primary/30 hover:text-primary"
                                  >
                                    <FileText className="h-3.5 w-3.5 shrink-0" />
                                    <span className="truncate">{artifact.name}</span>
                                  </a>
                                ))}
                              </div>
                            ) : (
                              <p className="mt-2 text-[11px] text-muted-foreground">
                                {t(
                                  'digitalAvatar.workspace.missionDeliverablesEmpty',
                                  '当前还没有可下载交付物，完成后会在这里展示最新产出。'
                                )}
                              </p>
                            )}
                          </div>
                        </div>
                      )}
                      <div className="mt-3 grid gap-2 lg:grid-cols-2">
                        {workspaceMissionsLoading ? (
                          <div className="rounded-md border border-dashed border-border/60 bg-background/70 px-3 py-4 text-[11px] text-muted-foreground">
                            {t('common.loading', '加载中...')}
                          </div>
                        ) : workspaceMissions.slice(0, 4).map((mission) => (
                          <div key={mission.mission_id} className="rounded-md border border-border/60 bg-background/80 p-3">
                            <div className="flex items-start justify-between gap-3">
                              <div className="min-w-0">
                                <p className="truncate text-xs font-medium text-foreground">{mission.goal}</p>
                                <p className="mt-1 text-[10px] text-muted-foreground">
                                  {t('digitalAvatar.workspace.missionProgress', '步骤 {{done}} / {{total}}', {
                                    done: mission.completed_steps,
                                    total: mission.step_count || 0,
                                  })}
                                  {' · '}
                                  {formatRelativeTime(mission.updated_at)}
                                </p>
                              </div>
                              <span className={`shrink-0 rounded-full border px-2 py-1 text-[10px] ${missionStatusBadgeClass(mission.status)}`}>
                                {t(`missions.status.${mission.status}` as const, mission.status)}
                              </span>
                            </div>
                            <div className="mt-3 flex flex-wrap gap-2">
                              <Button
                                variant="outline"
                                size="sm"
                                className="h-8 px-2.5 text-[11px]"
                                onClick={() => handleOpenMissionDetail(mission.mission_id)}
                              >
                                {t('digitalAvatar.workspace.openMissionDetail', '查看进度')}
                              </Button>
                              {(mission.status === 'failed' || mission.status === 'paused') && (
                                <Button
                                  size="sm"
                                  className="h-8 px-2.5 text-[11px]"
                                  onClick={() => handleResumeWorkspaceMission(mission.mission_id)}
                                >
                                  {t('digitalAvatar.workspace.resumeMission', '继续处理')}
                                </Button>
                              )}
                            </div>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                      </div>
                    </DialogContent>
                  </Dialog>
                  <div className={`min-h-0 flex-1 overflow-hidden ${focusMode ? 'pt-0' : ''}`}>
                    <ChatConversation
                      sessionId={managerSessionId}
                      agentId={effectiveManagerAgentId}
                      agentName={getAgentDisplayName(managerAgent, t('digitalAvatar.labels.managerAgent'))}
                      agent={managerAgent || undefined}
                      headerVariant={focusMode || workspaceChromeCollapsed ? 'compact' : 'default'}
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

          {!focusMode && inspectorOpen && (
            <>
              <button
                type="button"
                aria-label={t('digitalAvatar.actions.hideConsole', '收起控制台')}
                className="fixed inset-0 z-30 bg-black/20 lg:hidden"
                onClick={() => setInspectorOpen(false)}
              />
              <div className="fixed inset-y-0 right-0 z-40 w-[min(92vw,380px)] lg:absolute lg:inset-y-0 lg:right-0 lg:w-[360px]">
                <div className="flex h-full flex-col overflow-hidden rounded-xl border bg-card shadow-xl">
                  <div className="flex items-center justify-between gap-2 border-b px-3 py-2">
                    <div className="min-w-0">
                      <p className="text-[10px] text-muted-foreground">
                        {t('digitalAvatar.actions.consoleLabel', '侧边控制台')}
                      </p>
                      <p className="truncate text-sm font-medium text-foreground">
                        {t(`digitalAvatar.inspector.${inspectorTab}` as const, inspectorTab)}
                      </p>
                    </div>
                    <Button variant="outline" size="sm" onClick={() => setInspectorOpen(false)}>
                      {t('digitalAvatar.actions.hideConsole', '收起控制台')}
                    </Button>
                  </div>
                  <div className="min-h-0 flex-1 overflow-hidden p-3">
                    <div className="min-h-0 h-full flex flex-col">
            <div className="mb-2 flex flex-wrap gap-1.5">
              {(['overview', 'permissions', 'governance', 'logs', 'publish'] as InspectorTab[]).map((value) => (
                <button
                  key={value}
                  className={`rounded-md border px-2.5 py-1 text-[11px] transition-colors ${
                    inspectorTab === value
                      ? 'border-primary/50 bg-primary/10 text-primary'
                      : 'border-border/60 bg-background text-muted-foreground hover:text-foreground'
                  }`}
                  onClick={() => openInspectorPanel(value)}
                >
                  {t(`digitalAvatar.inspector.${value}` as const, value)}
                </button>
              ))}
            </div>
            <div className="min-h-0 overflow-y-auto space-y-3">
            <Card className={inspectorTab === 'overview' && selectedAvatar ? '' : 'hidden'}>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <Bot className="w-4 h-4" />
                    {t('digitalAvatar.workspace.managerWorkbenchPanelTitle', {
                      defaultValue: '工作面板',
                    })}
                  </span>
                  {selectedAvatar && hasWorkbenchContent ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-[11px]"
                      onClick={() => navigate(`/teams/${teamId}/digital-avatars/${selectedAvatar.id}/timeline`)}
                    >
                      {t('digitalAvatar.workspace.openTimeline', '查看时间线')}
                    </Button>
                  ) : null}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                {managerReportRows.length > 0 ? (
                  <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                    <div className="flex items-center justify-between gap-2">
                      <p className="text-[11px] font-medium text-foreground">
                        {t('digitalAvatar.workspace.managerReportTitle', { defaultValue: '最近汇报' })}
                      </p>
                      {managerReportRows.length > 2 ? (
                        <span className="text-[10px] text-muted-foreground">
                          {t('digitalAvatar.workspace.moreManagerReports', {
                            defaultValue: '共 {{count}} 条',
                            count: managerReportRows.length,
                          })}
                        </span>
                      ) : null}
                    </div>
                    {managerReportRows.slice(0, 3).map((row) => (
                      <div key={row.id} className="rounded-md border border-border/60 bg-background px-2.5 py-2">
                        <div className="flex items-start justify-between gap-2">
                          <div className="min-w-0">
                            <div className="flex flex-wrap items-center gap-1.5">
                              <span className="text-[11px] font-medium text-foreground">{row.title}</span>
                              <span className="rounded-full border border-border/60 bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                {row.kind === 'delivery'
                                  ? t('digitalAvatar.workspace.reportKindDelivery', { defaultValue: '交付' })
                                  : row.kind === 'progress'
                                    ? t('digitalAvatar.workspace.reportKindProgress', { defaultValue: '进展' })
                                    : row.kind === 'runtime'
                                      ? t('digitalAvatar.workspace.reportKindRuntime', { defaultValue: '运行' })
                                      : t('digitalAvatar.workspace.reportKindGovernance', { defaultValue: '治理' })}
                              </span>
                              <span className="rounded-full border border-border/60 bg-background px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                {t('digitalAvatar.workspace.reportSourceLabel', { defaultValue: '汇报方' })} {row.source}
                              </span>
                              {row.needsDecision ? (
                                <span className="rounded-full border border-amber-200 bg-amber-50 px-1.5 py-0.5 text-[10px] text-amber-700">
                                  {t('digitalAvatar.workspace.reportNeedsDecision', { defaultValue: '需决策' })}
                                </span>
                              ) : null}
                            </div>
                            <p className="mt-1 text-[11px] leading-5 text-muted-foreground">{row.summary}</p>
                            {row.recommendation ? (
                              <p className="mt-1 text-[11px] leading-5 text-foreground/80">{row.recommendation}</p>
                            ) : null}
                            {row.workObjects.length > 0 ? (
                              <div className="mt-2 flex flex-wrap items-center gap-1.5">
                                <span className="text-[10px] text-muted-foreground">
                                  {t('digitalAvatar.workspace.reportWorkObjects', { defaultValue: '涉及对象' })}
                                </span>
                                {row.workObjects.slice(0, 2).map((label) => (
                                  <span
                                    key={`${row.id}-workbench-work-${label}`}
                                    className="inline-flex max-w-[160px] truncate rounded-full border border-border/60 bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
                                    title={label}
                                  >
                                    {label}
                                  </span>
                                ))}
                                {row.workObjects.length > 2 ? (
                                  <span className="text-[10px] text-muted-foreground">+{row.workObjects.length - 2}</span>
                                ) : null}
                              </div>
                            ) : null}
                          </div>
                          <span className="shrink-0 text-[10px] text-muted-foreground">
                            {formatRelativeTime(row.ts)}
                          </span>
                        </div>
                        {row.actionLabel && row.action ? (
                          <div className="mt-2 flex justify-end">
                            <Button
                              variant="outline"
                              size="sm"
                              className="h-7 px-2.5 text-[11px]"
                              onClick={row.action}
                            >
                              {row.actionLabel}
                            </Button>
                          </div>
                        ) : null}
                      </div>
                    ))}
                  </div>
                ) : null}

                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-[11px] font-medium text-foreground">
                      {t('digitalAvatar.workspace.decisionWorkbenchTitle', { defaultValue: '待我决策' })}
                    </p>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-[11px]"
                      onClick={() => openInspectorPanel('governance')}
                    >
                      {t('digitalAvatar.workspace.openGovernanceConsole', '打开治理台')}
                    </Button>
                  </div>
                  {decisionWorkbenchItems.length > 0 ? (
                    <div className="space-y-2">
                      {decisionWorkbenchItems.slice(0, 3).map((item) => (
                        <div key={item.id} className="rounded-md border border-border/60 bg-background px-2.5 py-2">
                          <div className="flex items-start justify-between gap-2">
                            <div className="min-w-0">
                              <div className="flex flex-wrap items-center gap-1.5">
                                <span className="text-[11px] font-medium text-foreground">{item.title}</span>
                                <span className={`rounded-full border px-1.5 py-0.5 text-[10px] ${
                                  item.risk === 'high'
                                    ? 'border-red-200 bg-red-50 text-red-600'
                                    : item.risk === 'medium'
                                      ? 'border-amber-200 bg-amber-50 text-amber-700'
                                      : 'border-emerald-200 bg-emerald-50 text-emerald-700'
                                }`}>
                                  {item.risk === 'high'
                                    ? t('digitalAvatar.risk.high', '高风险')
                                    : item.risk === 'medium'
                                      ? t('digitalAvatar.risk.medium', '中风险')
                                      : t('digitalAvatar.risk.low', '低风险')}
                                </span>
                                <span className="rounded-full border border-border/60 bg-background px-1.5 py-0.5 text-[10px] text-muted-foreground">
                                  {item.status}
                                </span>
                              </div>
                              <p className="mt-1 text-[11px] leading-5 text-muted-foreground">{item.detail}</p>
                              {item.recommendation ? (
                                <p className="mt-1 text-[11px] leading-5 text-foreground/80">{item.recommendation}</p>
                              ) : null}
                            </div>
                          </div>
                          <div className="mt-2 flex justify-end">
                            <Button
                              variant="outline"
                              size="sm"
                              className="h-7 px-2.5 text-[11px]"
                              onClick={item.action}
                            >
                              {item.actionLabel}
                            </Button>
                          </div>
                        </div>
                      ))}
                    </div>
                  ) : (
                    <p className="text-[11px] leading-5 text-muted-foreground">
                      {t('digitalAvatar.workspace.decisionWorkbenchEmpty', {
                        defaultValue: '当前没有待管理 Agent 决策的事项。新的提权、优化建议和异常恢复会先汇总到这里。',
                      })}
                    </p>
                  )}
                </div>

                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-[11px] font-medium text-foreground">
                      {t('digitalAvatar.workspace.objectsTitle', { defaultValue: '工作对象' })}
                    </p>
                    <span className="rounded-full border border-border/60 bg-background px-2 py-1 text-[10px] text-muted-foreground">
                      {selectedAvatarBoundDocCount}
                    </span>
                  </div>
                  {workspaceObjectSummaries.length > 0 ? (
                    <div className="flex flex-wrap gap-2">
                      {workspaceObjectSummaries.slice(0, 6).map(({ doc, kind, kindLabel }) => (
                        <button
                          key={doc.id}
                          type="button"
                          className="flex min-w-[180px] max-w-full items-center justify-between gap-2 rounded-full border border-border/60 bg-background px-3 py-2 text-left text-[11px] hover:border-primary/30"
                          onClick={() => {
                            setWorkEntryMode('collaborate');
                            handleSelectWorkspaceDocument(doc.id).catch((error) => {
                              console.error('Failed to select workspace document:', error);
                            });
                          }}
                        >
                          <span className="min-w-0 flex-1 truncate text-foreground" title={doc.display_name || doc.name}>
                            {doc.display_name || doc.name}
                          </span>
                          <span className={`shrink-0 rounded-full border px-1.5 py-0.5 text-[10px] ${workspaceObjectKindBadgeClass(kind)}`}>
                            {kindLabel}
                          </span>
                        </button>
                      ))}
                      {workspaceObjectSummaries.length > 6 ? (
                        <span className="inline-flex items-center rounded-full border border-border/60 bg-background px-3 py-2 text-[11px] text-muted-foreground">
                          +{workspaceObjectSummaries.length - 6}
                        </span>
                      ) : null}
                    </div>
                  ) : (
                    <p className="text-[11px] leading-5 text-muted-foreground">
                      {t('digitalAvatar.workspace.objectsPanelEmpty', {
                        defaultValue: '当前岗位还没有绑定工作对象。先让管理 Agent 明确对象范围，或先去文件区准备材料。',
                      })}
                    </p>
                  )}
                </div>
              </CardContent>
            </Card>

            <Card className={inspectorTab === 'overview' ? '' : 'hidden'}>
              <CardContent className="py-3">
                <div className="flex items-start justify-between gap-3 rounded-md border bg-muted/20 px-3 py-2.5">
                  <div className="min-w-0">
                    <p className="text-xs font-medium text-foreground">
                      {t('digitalAvatar.workspace.protocolInlineTitle', '管理协议与审批规则已启用')}
                    </p>
                    <p className="mt-1 text-caption text-muted-foreground">
                      {t(
                        'digitalAvatar.workspace.protocolInlineHint',
                        '创建、提权、优化与发布都遵循同一套管理 Agent 协议。需要完整说明时，切到“使用指南”查看。'
                      )}
                    </p>
                  </div>
                  <Button variant="outline" size="sm" className="shrink-0" onClick={() => setTab('guide')}>
                    {t('digitalAvatar.tabs.guide', '使用指南')}
                  </Button>
                </div>
              </CardContent>
            </Card>

            <Card className={inspectorTab === 'overview' ? '' : 'hidden'}>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between">
                  <span className="flex items-center gap-1.5">
                    <Bot className="w-4 h-4" />
                    {t('digitalAvatar.workspace.capabilityTitle')}
                  </span>
                  {savingGovernance && <Loader2 className="w-3.5 h-3.5 animate-spin text-muted-foreground" />}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-caption">
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.managerAgent')}</p>
                  <p className="mt-0.5 text-xs font-medium">{getAgentName(agents, effectiveManagerAgentId, t('digitalAvatar.labels.unset'))}</p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.serviceAgent')}</p>
                  <p className="mt-0.5 text-xs font-medium">{getAgentName(agents, selectedAvatarServiceAgentId, t('digitalAvatar.labels.unset'))}</p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.documentAccess')}</p>
                  <p className="mt-0.5 text-xs font-medium">{formatDocumentAccessMode(selectedAvatarDocumentAccessMode, t)}</p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.list.lastActivity', '最近活动')}</p>
                  <p
                    className="mt-0.5 text-xs font-medium"
                    title={selectedAvatar?.updatedAt ? formatDateTime(selectedAvatar.updatedAt) : undefined}
                  >
                    {selectedAvatarLastActivityLabel}
                  </p>
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.allowedExtensions')}</p>
                  {renderCapabilityChipList(
                    selectedAvatarEffectiveExtensionEntries,
                    t('digitalAvatar.labels.unset'),
                  )}
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.allowedSkills')}</p>
                  {renderCapabilityChipList(
                    selectedAvatarEffectiveSkillEntries,
                    t('digitalAvatar.labels.unset'),
                  )}
                </div>
                <div className="rounded-md border bg-muted/25 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.workspace.capabilityScope', '能力开放范围')}</p>
                  <p className="mt-0.5 text-xs text-muted-foreground">{selectedAvatarCapabilityScopeHint}</p>
                </div>
                <Button variant="outline" size="sm" className="w-full" onClick={openLaboratory}>
                  <ExternalLink className="w-3.5 h-3.5 mr-1" />
                  {t('digitalAvatar.actions.openLaboratory')}
                </Button>
              </CardContent>
            </Card>

            <Card className={inspectorTab === 'permissions' ? '' : 'hidden'}>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <ShieldAlert className="w-4 h-4" />
                  {t('digitalAvatar.workspace.resourceAndAccess', '资源与权限')}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 text-caption">
                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-muted-foreground">{t('digitalAvatar.workspace.boundDocs', '绑定文档')}</p>
                    {canManage ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-auto px-0 text-xs text-primary"
                        onClick={() => setShowPermissionDocPicker(true)}
                      >
                        + {t('common.edit', '编辑')}
                      </Button>
                    ) : null}
                  </div>
                  {permissionSelectedDocuments.length > 0 ? (
                    <div className="flex flex-wrap gap-1.5">
                      {permissionSelectedDocuments.map((doc) => (
                        <span
                          key={doc.id}
                          className="inline-flex max-w-full items-center rounded-full border border-border/60 bg-background px-2 py-1 text-xs text-foreground"
                          title={doc.display_name || doc.name}
                        >
                          <span className="truncate">{doc.display_name || doc.name}</span>
                        </span>
                      ))}
                    </div>
                  ) : (
                    <p className="text-xs text-muted-foreground">
                      {t('digitalAvatar.workspace.noBoundDocuments', '暂无可用文档')}
                    </p>
                  )}
                </div>
                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <p className="text-muted-foreground">{t('digitalAvatar.labels.documentAccess')}</p>
                  {canManage ? (
                    <select
                      value={permissionDocumentAccessMode}
                      onChange={(event) => setPermissionDocumentAccessMode(event.target.value as PortalDocumentAccessMode)}
                      className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                    >
                      <option value="read_only">{t('digitalAvatar.documentAccess.readOnly', '只读')}</option>
                      <option value="co_edit_draft">{t('digitalAvatar.documentAccess.coEditDraft', '协作草稿')}</option>
                      <option value="controlled_write">{t('digitalAvatar.documentAccess.controlledWrite', '受控写入')}</option>
                    </select>
                  ) : (
                    <p className="text-xs font-medium">{formatDocumentAccessMode(permissionDocumentAccessMode, t)}</p>
                  )}
                  <p className="text-[11px] text-muted-foreground">
                    {t(
                      'digitalAvatar.workspace.documentAccessHint',
                      '访客可读写文档，写入行为受策略控制。',
                    )}
                  </p>
                </div>
                <div className="rounded-md border bg-muted/20 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.workspace.permissionPreviewTitle', '生效权限预览')}</p>
                  <div className="mt-1 space-y-1 text-[11px] text-muted-foreground">
                    {permissionPreviewDraft.map((line: string) => (
                      <p key={line}>{line}</p>
                    ))}
                  </div>
                </div>
                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-muted-foreground">
                      {t('digitalAvatar.workspace.allowedVisitorExtensions', '允许扩展（访客）')}
                    </p>
                    {canManage ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-auto px-0 text-xs text-primary"
                        onClick={() => setPermissionSelectorDialog('extensions')}
                      >
                        + {t('common.edit', '编辑')}
                      </Button>
                    ) : null}
                  </div>
                  {renderCapabilityChipList(
                    permissionSelectedExtensionEntries,
                    t('digitalAvatar.labels.unset'),
                  )}
                </div>
                <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <p className="text-muted-foreground">
                      {t('digitalAvatar.workspace.allowedVisitorSkills', '允许技能（访客）')}
                    </p>
                    {canManage ? (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-auto px-0 text-xs text-primary"
                        onClick={() => setPermissionSelectorDialog('skills')}
                      >
                        + {t('common.edit', '编辑')}
                      </Button>
                    ) : null}
                  </div>
                  {renderCapabilityChipList(
                    permissionSelectedSkillEntries,
                    t('digitalAvatar.labels.unset'),
                  )}
                </div>
                <div className="rounded-md border bg-muted/20 p-2.5">
                  <p className="text-muted-foreground">{t('digitalAvatar.workspace.capabilityScope', '能力开放范围')}</p>
                  <p className="mt-1 text-[11px] text-muted-foreground">{permissionScopeHint}</p>
                </div>
                {canManage ? (
                  <Button
                    type="button"
                    size="sm"
                    className="w-full"
                    onClick={handleSaveAvatarPermissions}
                    disabled={savingPermissionConfig}
                  >
                    {savingPermissionConfig ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : null}
                    {t('digitalAvatar.actions.savePermissionConfig', '保存权限配置')}
                  </Button>
                ) : null}
              </CardContent>
            </Card>

            <DocumentPicker
              teamId={teamId}
              open={showPermissionDocPicker}
              onClose={() => setShowPermissionDocPicker(false)}
              multiple
              selectedIds={permissionSelectedDocIds}
              selectedDocuments={permissionSelectedDocuments}
              onSelect={(docs) => {
                setPermissionSelectedDocIds(docs.map((doc) => doc.id));
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
                              onClick={() => setPermissionSelectedExtensions((current) => toggleSelection(current, option.id))}
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
                              onClick={() => setPermissionSelectedSkillIds((current) => toggleSelection(current, entry.id))}
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

            <Card className={inspectorTab === 'governance' ? '' : 'hidden'}>
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
                      className="h-7 min-w-[130px] flex-1 rounded border bg-background px-2 text-[11px]"
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
                            {item.kind === 'proposal'
                              ? t(`digitalAvatar.governance.proposalStatus.${toProposalStatusLabel(item.status as ProposalStatus)}`, item.status)
                              : item.kind === 'ticket'
                              ? t(`digitalAvatar.governance.ticketStatus.${toOptimizationStatusLabel(item.status as OptimizationStatus)}`, item.status)
                              : t(`digitalAvatar.governance.status.${item.status}`, item.status)}
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

            <Card className={inspectorTab === 'logs' ? '' : 'hidden'}>
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
                            {item.rowType === 'runtime'
                              ? t(`digitalAvatar.governance.runtimeStatus.${item.status}`, item.status)
                              : item.rowType === 'proposal'
                              ? t(`digitalAvatar.governance.proposalStatus.${toProposalStatusLabel(item.status as ProposalStatus)}`, item.status)
                              : item.rowType === 'ticket'
                              ? t(`digitalAvatar.governance.ticketStatus.${toOptimizationStatusLabel(item.status as OptimizationStatus)}`, item.status)
                              : t(`digitalAvatar.governance.status.${item.status}`, item.status)}
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
                          <span className="text-[10px] text-muted-foreground">{item.status}</span>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card className={inspectorTab === 'logs' ? '' : 'hidden'}>
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
                        <span className="text-[10px] text-muted-foreground">{item.status}</span>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            <Card className={inspectorTab === 'logs' ? '' : 'hidden'}>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center justify-between gap-2">
                  <span className="flex items-center gap-1.5">
                    <Clock3 className="w-4 h-4" />
                    {t('digitalAvatar.governance.runtimeEventsTitle', '完整运行日志（可追溯）')}
                  </span>
                  <div className="flex items-center gap-1.5">
                    <span className="text-[10px] text-muted-foreground">
                      {t('digitalAvatar.governance.runtimeEventsCount', '事件 {{count}}', {
                        count: persistedEvents.length,
                      })}
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
                      onClick={loadOlderPersistedEvents}
                      disabled={!persistedEventsHasMore || persistedEventsLoadingMore || persistedEventsLoading}
                    >
                      {persistedEventsLoadingMore ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        t('digitalAvatar.governance.runtimeEventsLoadOlder', '加载更早')
                      )}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[10px]"
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
                </CardTitle>
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
                    className="h-7 min-w-[120px] flex-1 rounded border bg-background px-2 text-[11px]"
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
                      <div key={`${event.run_id || 'no_run'}:${event.event_id}:${event.created_at}`} className={`rounded-md border p-2 ${severityClass(severity)}`}>
                        <div className="flex items-center justify-between gap-2">
                          <div className="min-w-0">
                            <p className="text-[11px] font-medium truncate">
                              #{event.event_id} · {event.event_type}
                            </p>
                            <p className="text-[10px] text-muted-foreground truncate">
                              {event.run_id || 'run:unknown'} · {new Date(event.created_at).toLocaleString()}
                            </p>
                          </div>
                          <span className={`px-1.5 py-0.5 rounded text-[10px] border ${badgeClass(severity === 'error' ? 'rejected' : severity === 'warn' ? 'pending' : 'approved')}`}>
                            {eventTypeBadge(event.event_type)}
                          </span>
                        </div>
                        <p className="mt-1 text-caption text-muted-foreground whitespace-pre-wrap break-words">
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

            <Card className={inspectorTab === 'governance' ? '' : 'hidden'}>
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
                              {item.kind === 'proposal'
                                ? t(`digitalAvatar.governance.proposalStatus.${toProposalStatusLabel(item.status as ProposalStatus)}`, item.status)
                                : item.kind === 'ticket'
                                ? t(`digitalAvatar.governance.ticketStatus.${toOptimizationStatusLabel(item.status as OptimizationStatus)}`, item.status)
                                : t(`digitalAvatar.governance.status.${item.status}`, item.status)}
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
                              {t(`digitalAvatar.governance.runtimeStatus.${item.status}`, item.status)}
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
            <Card className={inspectorTab === 'publish' ? '' : 'hidden'}>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm flex items-center gap-1.5">
                  <ExternalLink className="w-4 h-4" />
                  {t('digitalAvatar.workspace.publishTitle', '发布视图')}
                </CardTitle>
                <p className="text-caption text-muted-foreground">
                  {t('digitalAvatar.workspace.publishHint', '面向访客的入口信息、边界与预览入口。')}
                </p>
              </CardHeader>
              <CardContent className="space-y-2 text-caption">
                {!selectedAvatar ? (
                  <p className="text-muted-foreground">{t('digitalAvatar.states.noAvatarSelected', '未选择分身')}</p>
                ) : (
                    <>
                      <div className="rounded-md border bg-muted/20 p-2.5 space-y-1.5">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div>
                            <p className="text-muted-foreground">
                              {t('digitalAvatar.workspace.publishControlTitle', '对外发布控制')}
                            </p>
                            <p className="mt-0.5 text-[11px] text-muted-foreground">
                              {selectedAvatarStatus === 'published'
                                ? selectedAvatarEffectivePublicConfig?.publicAccessEnabled
                                  ? t('digitalAvatar.workspace.publishControlPublishedHint', '当前已发布，访客可通过正式入口访问。')
                                  : t('digitalAvatar.workspace.publishControlPreviewOnlyHint', '当前处于已发布但仅管理预览状态，不会对外暴露正式访客页。')
                                : selectedAvatarStatus === 'archived'
                                ? t('digitalAvatar.workspace.publishControlArchivedHint', '当前已归档，重新发布后才会恢复访客访问。')
                                : t('digitalAvatar.workspace.publishControlDraftHint', '草稿状态下，访客页不可访问，仅可通过管理预览或测试入口验证。')}
                            </p>
                          </div>
                          {canManage ? (
                            <Button size="sm" onClick={handleToggleAvatarPublish} disabled={publishingAvatar}>
                              {selectedAvatarStatus === 'published'
                                ? t('digitalAvatar.actions.unpublishAvatar', '停止对外服务')
                                : t('digitalAvatar.actions.publishAvatar', '发布分身')}
                            </Button>
                          ) : null}
                        </div>
                      </div>
                    <div className="rounded-md border bg-muted/20 p-2.5">
                      <p className="text-muted-foreground">{t('digitalAvatar.workspace.summaryPublish', '发布地址')}</p>
                      <p className="mt-0.5 text-xs font-medium break-all">
                          {selectedAvatarPublicUrl || (
                            selectedAvatarStatus === 'published'
                              ? t('digitalAvatar.workspace.previewOnly', '仅管理预览')
                              : t('digitalAvatar.workspace.unpublished', '未发布')
                          )}
                        </p>
                    </div>
                    <div className="rounded-md border bg-muted/20 p-2.5">
                      <p className="text-muted-foreground">{t('digitalAvatar.workspace.summaryStatus', '当前状态')}</p>
                      <p className="mt-0.5 text-xs font-medium">{selectedAvatarStatusLabel}</p>
                    </div>
                    <div className="rounded-md border bg-muted/20 p-2.5">
                      <p className="text-muted-foreground">{t('digitalAvatar.workspace.summaryType', '分身类型')}</p>
                      <div className="mt-1">
                        <AvatarTypeBadge type={selectedAvatarType} />
                      </div>
                    </div>
                    <div className="rounded-md border bg-muted/20 p-2.5">
                      <p className="text-muted-foreground">{t('common.description', '描述')}</p>
                      <p className="mt-0.5 text-xs font-medium break-words">
                        {selectedAvatar.description || t('digitalAvatar.labels.unset')}
                      </p>
                    </div>
                    <div className="rounded-md border bg-muted/20 p-2.5 space-y-2">
                      <p className="text-muted-foreground">{t('digitalAvatar.workspace.publicConfigTitle', '对外配置与生效')}</p>
                      <div className="grid gap-2 sm:grid-cols-2">
                        <div className="rounded-md border bg-background/80 p-2">
                          <p className="text-[11px] text-muted-foreground">{t('digitalAvatar.workspace.outputFormLabel', '配置输出形态')}</p>
                          <p className="mt-0.5 text-xs font-medium">{selectedAvatarOutputFormLabel}</p>
                        </div>
                        <div className="rounded-md border bg-background/80 p-2">
                          <p className="text-[11px] text-muted-foreground">{t('digitalAvatar.workspace.effectiveExposureLabel', '生效对外曝光')}</p>
                          <p className="mt-0.5 text-xs font-medium">{selectedAvatarExposureLabel}</p>
                        </div>
                        <div className="rounded-md border bg-background/80 p-2">
                          <p className="text-[11px] text-muted-foreground">{t('digitalAvatar.workspace.chatWidgetConfigLabel', '聊天挂件配置')}</p>
                          <p className="mt-0.5 text-xs font-medium">
                            {selectedAvatarShowChatWidget
                              ? t('common.enabled', '已开启')
                              : t('common.disabled', '已关闭')}
                          </p>
                        </div>
                        <div className="rounded-md border bg-background/80 p-2">
                          <p className="text-[11px] text-muted-foreground">{t('digitalAvatar.workspace.chatWidgetEffectiveLabel', '聊天入口生效结果')}</p>
                          <p className="mt-0.5 text-xs font-medium">{selectedAvatarChatWidgetEffectLabel}</p>
                        </div>
                      </div>
                      {canManage ? (
                        <div className="flex flex-wrap gap-2">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={handleToggleChatWidget}
                            disabled={updatingPublicConfig}
                          >
                            {updatingPublicConfig ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> : null}
                            {selectedAvatarShowChatWidget
                              ? t('digitalAvatar.workspace.hideChatWidget', '关闭默认聊天挂件')
                              : t('digitalAvatar.workspace.showChatWidget', '开启默认聊天挂件')}
                          </Button>
                        </div>
                      ) : null}
                    </div>
                    <div className="rounded-md border bg-muted/20 p-2.5 space-y-1.5">
                      <p className="text-muted-foreground">{t('digitalAvatar.workspace.permissionPreviewTitle', '当前权限效果')}</p>
                      <div className="space-y-1 text-[11px] text-muted-foreground">
                        {permissionPreview.map((line: string) => (
                          <p key={line}>{line}</p>
                        ))}
                      </div>
                      <div className="space-y-2">
                        <div>
                          <p className="text-[11px] text-muted-foreground">
                            {t('digitalAvatar.workspace.activeExt', '生效扩展')}
                          </p>
                          {renderCapabilityChipList(
                            selectedAvatarEffectiveExtensionEntries,
                            t('digitalAvatar.labels.unset'),
                          )}
                        </div>
                        <div>
                          <p className="text-[11px] text-muted-foreground">
                            {t('digitalAvatar.workspace.activeSkills', '生效技能')}
                          </p>
                          {renderCapabilityChipList(
                            selectedAvatarEffectiveSkillEntries,
                            t('digitalAvatar.labels.unset'),
                          )}
                        </div>
                      </div>
                      <p className="text-[10px] text-muted-foreground">{selectedAvatarCapabilityScopeHint}</p>
                    </div>
                    <div className="rounded-md border bg-muted/20 p-2.5 space-y-1.5">
                      <div className="flex flex-wrap items-center gap-1.5">
                        {availablePublishModes.map((mode) => (
                          <button
                            key={mode}
                            type="button"
                            className={`rounded border px-2 py-1 text-[11px] ${
                              publishViewMode === mode
                                ? 'border-primary/50 bg-primary/10 text-primary'
                                : 'border-border/60 bg-background text-muted-foreground hover:text-foreground'
                            }`}
                            onClick={() => setPublishViewMode(mode)}
                          >
                            {mode === 'visitor'
                              ? t('digitalAvatar.workspace.publishMode.visitorTab', '访客视角')
                              : mode === 'preview'
                              ? t('digitalAvatar.workspace.publishMode.previewTab', '管理预览')
                              : t('digitalAvatar.workspace.publishMode.testTab', '测试入口')}
                          </button>
                        ))}
                      </div>
                      <div className="rounded-md border bg-background/80 p-2">
                        <div className="flex items-center justify-between gap-2">
                          <div>
                            <p className="text-[11px] font-medium text-foreground">
                              {t('digitalAvatar.workspace.publishMode.compareTitle', '视角切换说明')}
                            </p>
                            <p className="text-[10px] text-muted-foreground">
                              {t('digitalAvatar.workspace.publishMode.compareHint', '访客页用于正式对外交付，管理预览用于内部验收，测试入口用于联调排查。')}
                            </p>
                          </div>
                        </div>
                        <div className="mt-2 grid gap-2 sm:grid-cols-3">
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
                                key={`compare-${mode}`}
                                className={`rounded-md border p-2 ${
                                  publishViewMode === mode
                                    ? 'border-primary/50 bg-primary/5'
                                    : 'border-border/60 bg-muted/10'
                                }`}
                              >
                                <div className="flex items-center justify-between gap-2">
                                  <p className="text-[11px] font-medium text-foreground">{title}</p>
                                  {publishViewMode === mode ? (
                                    <span className="rounded-full border border-primary/40 bg-primary/10 px-1.5 py-0.5 text-[10px] text-primary">
                                      {t('digitalAvatar.workspace.publishMode.currentMode', '当前')}
                                    </span>
                                  ) : null}
                                </div>
                                <p className="mt-1 text-[10px] leading-5 text-muted-foreground">{desc}</p>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                      <p className="text-muted-foreground">{publishModeDescription.title}</p>
                      <p className="text-[11px] text-muted-foreground">{publishModeDescription.description}</p>
                      <div className="space-y-1 text-[11px] text-muted-foreground">
                        {publishModeDescription.bullets.map((bullet) => (
                          <p key={bullet}>{bullet}</p>
                        ))}
                      </div>
                    </div>
                    {activePublishUrl && (
                      <div className="rounded-md border bg-muted/20 p-2.5">
                        <p className="text-muted-foreground">
                          {publishViewMode === 'visitor'
                            ? t('digitalAvatar.workspace.publishMode.visitorAddress', '访客入口')
                            : publishViewMode === 'preview'
                            ? t('digitalAvatar.workspace.previewUrl', '管理预览')
                            : t('digitalAvatar.workspace.testUrl', '测试入口')}
                        </p>
                        <p className="mt-0.5 text-xs font-medium break-all">{activePublishUrl}</p>
                      </div>
                    )}
                    {secondaryPublishLinks.length > 0 && (
                      <div className="rounded-md border bg-muted/20 p-2.5 space-y-1.5">
                        <p className="text-muted-foreground">{t('digitalAvatar.workspace.otherPublishLinks', '其他可用入口')}</p>
                        {secondaryPublishLinks.map((link) => (
                          <div key={link.key} className="rounded-md border bg-background/80 p-2">
                            <p className="text-[11px] text-muted-foreground">{link.label}</p>
                            <p className="mt-0.5 text-xs font-medium break-all">{link.url}</p>
                          </div>
                        ))}
                      </div>
                    )}
                    <div className="flex flex-wrap gap-2">
                      {canManage ? (
                        <Button size="sm" className="flex-1 min-w-[120px]" onClick={handleToggleAvatarPublish} disabled={publishingAvatar}>
                          {selectedAvatarStatus === 'published'
                            ? t('digitalAvatar.actions.unpublishAvatar', '停止对外服务')
                            : t('digitalAvatar.actions.publishAvatar', '发布分身')}
                        </Button>
                      ) : null}
                      <Button
                        variant="outline"
                        size="sm"
                        className="flex-1 min-w-[120px]"
                        onClick={() => activePublishUrl && window.open(activePublishUrl, '_blank', 'noopener,noreferrer')}
                        disabled={!activePublishUrl}
                      >
                        <ExternalLink className="w-3.5 h-3.5 mr-1" />
                        {publishViewMode === 'visitor'
                          ? t('digitalAvatar.workspace.openPublicPage', '打开访客页')
                          : publishViewMode === 'preview'
                          ? t('digitalAvatar.workspace.openPreviewPage', '打开管理预览')
                          : t('digitalAvatar.workspace.openTestPage', '打开测试入口')}
                      </Button>
                      {selectedAvatarPreviewUrl && publishViewMode !== 'preview' ? (
                        <Button
                          variant="outline"
                          size="sm"
                          className="flex-1 min-w-[120px]"
                          onClick={() => window.open(selectedAvatarPreviewUrl, '_blank', 'noopener,noreferrer')}
                        >
                          <ExternalLink className="w-3.5 h-3.5 mr-1" />
                          {t('digitalAvatar.workspace.openPreviewPage', '打开管理预览')}
                        </Button>
                      ) : null}
                      <Button variant="outline" size="sm" className="flex-1 min-w-[120px]" onClick={openLaboratory}>
                        <ExternalLink className="w-3.5 h-3.5 mr-1" />
                        {t('digitalAvatar.actions.openLaboratory')}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        className="flex-1 min-w-[120px]"
                        onClick={() => selectedAvatar && navigate(`/teams/${teamId}/digital-avatars/${selectedAvatar.id}/timeline`)}
                        disabled={!selectedAvatar}
                      >
                        <ExternalLink className="w-3.5 h-3.5 mr-1" />
                        {t('digitalAvatar.timeline.openStandalone', '独立查看')}
                      </Button>
                    </div>
                  </>
                )}
              </CardContent>
            </Card>
                    </div>
                    </div>
                  </div>
                </div>
              </div>
            </>
          )}
          </div>
        </div>
        </>
      )}

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
