import { useCallback, useEffect, useMemo, useRef, useState, type ChangeEvent, type CSSProperties, type KeyboardEvent, type ReactNode, type SyntheticEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { createPortal } from 'react-dom';
import i18n from '../../i18n';
import {
  Archive,
  ArrowLeft,
  BellOff,
  Bot,
  Check,
  CheckCheck,
  ChevronRight,
  ClipboardList,
  Hash,
  Lightbulb,
  Loader2,
  Lock,
  MessageSquareReply,
  MoreHorizontal,
  Pin,
  PinOff,
  Paperclip,
  Plus,
  Save,
  Send,
  Sparkles,
  TriangleAlert,
  Upload,
  X,
  type LucideIcon,
} from 'lucide-react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { useIsMobile } from '../../hooks/useMediaQuery';
import {
  chatApi,
  type ChatChannelAgentAutonomyMode,
  type ChatChannelDetail,
  type DelegationRuntime,
  type DelegationRuntimeEventPayload,
  type ChatChannelDisplayKind,
  type ChatChannelDisplayStatus,
  type ChatChannelUserPrefs,
  type ChatChannelDeleteMode,
  type ChatChannelMember,
  type ChatChannelMessage,
  type ChatChannelMessageSurface,
  type ChatChannelSummary,
  type ChatChannelThread,
  type ChatChannelType,
  type ChatChannelThreadState,
  type ChatChannelVisibility,
  type ChannelMention,
  type ComposerCapabilitiesCatalog,
} from '../../api/chat';
import type { TeamAgent } from '../../api/agent';
import type { DocumentSummary, FolderTreeNode } from '../../api/documents';
import { documentApi, folderApi } from '../../api/documents';
import { ChatMessageBubble, type ToolCallInfo } from '../chat/ChatMessageBubble';
import {
  ChatCapabilityPicker,
  type ChatCapabilitySelection,
} from '../chat/ChatCapabilityPicker';
import { DocumentPicker } from '../documents/DocumentPicker';
import { fetchVisibleChatAgents } from '../chat/visibleChatAgents';
import { apiClient } from '../../api/client';
import type { TeamMember } from '../../api/types';
import { useAuth } from '../../contexts/AuthContext';
import { useToast } from '../../contexts/ToastContext';

function bilingual(zh: string, en: string): string {
  const lang = i18n.resolvedLanguage || i18n.language || 'zh';
  return lang.toLowerCase().startsWith('en') ? en : zh;
}

type ChannelRenderMessage = ChatChannelMessage & {
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number; phase?: string; reason?: string };
  isStreaming?: boolean;
};

type ChannelDisplayView = 'work' | 'update';
type CollaborationStatusFilter = 'all' | 'proposed' | 'active' | 'awaiting_confirmation' | 'adopted' | 'rejected';
type CollaborationSurfaceFilter = 'all' | 'temporary' | 'issue';
type InspectorTabKey = 'documents' | 'ai_outputs' | 'members' | 'settings';
type SidePanelMode = InspectorTabKey | 'workspace' | 'thread';

interface TeamChannelsPanelProps {
  teamId: string;
  initialChannelId?: string | null;
  initialThreadRootId?: string | null;
}

interface ChannelFormState {
  name: string;
  description: string;
  visibility: ChatChannelVisibility;
  channelType: ChatChannelType;
  defaultAgentId: string;
  workspaceDisplayName: string;
  repoDefaultBranch: string;
  agentAutonomyMode: ChatChannelAgentAutonomyMode;
  channelGoal: string;
  participantNotes: string;
  expectedOutputs: string;
  collaborationStyle: string;
  memberUserIds: string[];
}

interface PendingCollaborationActionConfirm {
  title: string;
  description: string;
  confirmText: string;
  actionLabel: string;
  subject: string;
  note?: string;
  variant?: 'default' | 'destructive';
  onConfirm: () => Promise<void> | void;
}

function workspaceFileSupportsBrowserPreview(filePath: string): boolean {
  const lower = filePath.toLowerCase();
  return lower.endsWith('.html') || lower.endsWith('.htm');
}

type ComposerMentionComposer = 'root' | 'thread';

interface ComposerMentionOption {
  key: string;
  mention_type: 'member' | 'agent';
  target_id: string;
  label: string;
  insertLabel: string;
  subtitle: string;
}

interface ActiveComposerMention {
  composer: ComposerMentionComposer;
  query: string;
  start: number;
  end: number;
  options: ComposerMentionOption[];
  selectedIndex: number;
}

interface ComposerPickerOption {
  key: string;
  value: string;
  kind: 'default_agent' | 'agent' | 'member';
  label: string;
}

interface ActiveComposerPicker {
  composer: ComposerMentionComposer;
  options: ComposerPickerOption[];
  selectedIndex: number;
}

interface ComposerMenuViewport {
  top: number;
  left: number;
  width: number;
  placement: 'top' | 'bottom';
}

const FILE_ACCEPT = [
  '.pdf',
  '.doc',
  '.docx',
  '.xls',
  '.xlsx',
  '.ppt',
  '.pptx',
  '.txt',
  '.md',
  '.csv',
  '.json',
  '.xml',
  '.html',
  '.htm',
  '.js',
  '.ts',
  '.tsx',
  '.jsx',
  '.py',
  '.go',
  '.rs',
  '.java',
  '.c',
  '.cpp',
  '.h',
  '.hpp',
  '.css',
  '.scss',
  '.yaml',
  '.yml',
  '.log',
  '.png',
  '.jpg',
  '.jpeg',
  '.webp',
  '.gif',
  '.svg',
].join(',');

const CAPABILITY_BLOCK_HEADER = bilingual('请优先使用以下能力完成本轮任务：', 'Please prioritize using the following capabilities in this turn:');
const DEFAULT_CHANNEL_AGENT_VALUE = '__default_channel_agent__';

function buildCapabilityDraft(refs: string[], remainder: string): string {
  const block = refs.length
    ? `${CAPABILITY_BLOCK_HEADER}\n${refs.map((ref) => `- ${ref}`).join('\n')}`
    : '';
  const body = remainder.trimStart();
  if (block && body) {
    return `${block}\n\n${body}`;
  }
  return block || body;
}

function inferCapabilityNameFromRef(ref: string): string {
  const parts = ref
    .replace(/^\[\[/, '')
    .replace(/\]\]$/, '')
    .split('|');
  return parts[1] || ref;
}

function renderAiWorkbenchGroupLabel(group?: string | null): string {
  switch (group) {
    case 'draft':
      return bilingual('草稿', 'Draft');
    case 'report':
      return bilingual('报告', 'Report');
    case 'summary':
      return bilingual('总结', 'Summary');
    case 'review':
      return bilingual('审查', 'Review');
    case 'plan':
      return bilingual('计划', 'Plan');
    case 'research':
      return bilingual('研究', 'Research');
    case 'artifact':
      return bilingual('产物', 'Artifact');
    case 'code':
      return bilingual('代码', 'Code');
    default:
      return bilingual('其他', 'Other');
  }
}

function flattenFolders(nodes: FolderTreeNode[], level = 0): Array<{ path: string; label: string }> {
  const items: Array<{ path: string; label: string }> = [];
  for (const node of nodes) {
    items.push({
      path: node.fullPath,
      label: `${'  '.repeat(level)}${node.name}`,
    });
    if (node.children?.length) {
      items.push(...flattenFolders(node.children, level + 1));
    }
  }
  return items;
}

function emptyForm(defaultAgentId = ''): ChannelFormState {
  return {
    name: '',
    description: '',
    visibility: 'team_public',
    channelType: 'general',
    defaultAgentId,
    workspaceDisplayName: '',
    repoDefaultBranch: 'main',
    agentAutonomyMode: 'standard',
    channelGoal: '',
    participantNotes: '',
    expectedOutputs: '',
    collaborationStyle: '',
    memberUserIds: [],
  };
}

function channelVisibilityLabel(visibility: ChatChannelVisibility) {
  return visibility === 'team_private'
    ? bilingual('私密频道', 'Private channel')
    : bilingual('公开频道', 'Public channel');
}

function channelTypeLabel(channelType: ChatChannelType) {
  if (channelType === 'coding') return bilingual('编程项目频道', 'Coding project channel');
  if (channelType === 'scheduled_task') return bilingual('定时任务频道', 'Scheduled task channel');
  return bilingual('普通协作频道', 'General collaboration channel');
}

function workspaceLifecycleLabel(state?: string | null) {
  switch (state) {
    case 'detached':
      return bilingual('已解绑', 'Detached');
    case 'archived':
      return bilingual('已归档', 'Archived');
    case 'pending_delete':
      return bilingual('待删除', 'Pending deletion');
    case 'deleted':
      return bilingual('已删除', 'Deleted');
    case 'active_bound':
      return bilingual('使用中', 'Active');
    default:
      return bilingual('未登记', 'Untracked');
  }
}

function delegationRuntimeStatusTone(status?: string | null) {
  switch ((status || '').toLowerCase()) {
    case 'completed':
      return 'bg-status-success-bg text-status-success-text';
    case 'failed':
      return 'bg-status-error-bg text-status-error-text';
    case 'running':
      return 'bg-status-info-bg text-status-info-text';
    case 'pending':
      return 'bg-status-warning-bg text-status-warning-text';
    default:
      return 'bg-muted text-muted-foreground';
  }
}

function delegationRuntimeStatusLabel(status?: string | null) {
  switch ((status || '').toLowerCase()) {
    case 'completed':
      return bilingual('已完成', 'Completed');
    case 'failed':
      return bilingual('失败', 'Failed');
    case 'running':
      return bilingual('运行中', 'Running');
    case 'pending':
      return bilingual('等待中', 'Pending');
    default:
      return bilingual('空闲', 'Idle');
  }
}

function delegationWorkerRoleLabel(role?: string | null) {
  switch ((role || '').toLowerCase()) {
    case 'leader':
      return bilingual('协调者', 'Leader');
    case 'subagent':
      return bilingual('独立任务', 'Single worker');
    case 'swarm_worker':
      return bilingual('并行任务', 'Parallel worker');
    case 'validation_worker':
      return bilingual('验证任务', 'Validation worker');
    default:
      return bilingual('任务', 'Worker');
  }
}

function delegationWorkerTitle(worker: DelegationRuntime['workers'][number]) {
  const raw = (worker.title || worker.worker_id || '').trim();
  const index =
    raw.match(/^worker_(\d+)(?:_|$)/i)?.[1] ||
    (worker.worker_id || '').match(/^worker_(\d+)(?:_|$)/i)?.[1];
  if (index) {
    return `Worker ${index}`;
  }
  if ((!raw || raw === worker.worker_id) && worker.role === 'subagent') {
    return 'Worker';
  }
  return raw || 'Worker';
}

function delegationLeaderTitle(title?: string | null) {
  const raw = (title || '').trim();
  if (!raw || raw === 'Leader') {
    return bilingual('协调者', 'Leader');
  }
  return raw;
}

function buildDelegationRuntimeSummary(runtime: DelegationRuntime | null): string {
  if (!runtime) return bilingual('无委托', 'No delegation');
  const running = runtime.workers.filter((worker) => worker.status === 'running').length;
  const pending = runtime.workers.filter((worker) => worker.status === 'pending').length;
  const completed = runtime.workers.filter((worker) => worker.status === 'completed').length;
  const failed = runtime.workers.filter((worker) => worker.status === 'failed').length;
  if (running > 0) {
    return bilingual(
      `${running} worker(s) running${completed > 0 ? `, ${completed} completed` : ''}`,
      `${running} worker(s) running${completed > 0 ? `, ${completed} completed` : ''}`,
    );
  }
  if (pending > 0) {
    return bilingual(`${pending} 个 worker 等待中`, `${pending} worker(s) pending`);
  }
  if (failed > 0) {
    return completed > 0
      ? bilingual(
          `${completed} worker(s) completed, ${failed} failed`,
          `${completed} worker(s) completed, ${failed} failed`,
        )
      : bilingual(`${failed} 个 worker 失败`, `${failed} worker(s) failed`);
  }
  return bilingual(
    `${completed || runtime.workers.length} worker(s) completed`,
    `${completed || runtime.workers.length} worker(s) completed`,
  );
}

function applyDelegationRuntimePatch(
  previous: DelegationRuntime | null,
  payload: DelegationRuntimeEventPayload,
): DelegationRuntime {
  const base: DelegationRuntime = previous
    ? { ...previous, workers: [...previous.workers] }
    : {
        active_run: true,
        mode: payload.mode || 'subagent',
        status: payload.status || 'running',
        summary: payload.summary || null,
        leader: null,
        workers: [],
      };

  if (payload.mode) base.mode = payload.mode;
  if (payload.status) base.status = payload.status;
  if (typeof payload.summary === 'string') base.summary = payload.summary;

  if (payload.worker) {
    const workerIndex = base.workers.findIndex(
      (worker) => worker.worker_id === payload.worker!.worker_id,
    );
    if (workerIndex >= 0) {
      base.workers[workerIndex] = {
        ...base.workers[workerIndex],
        ...payload.worker,
        summary: payload.worker.summary ?? base.workers[workerIndex].summary ?? undefined,
        result_summary:
          payload.worker.result_summary ??
          base.workers[workerIndex].result_summary ??
          undefined,
        error: payload.worker.error ?? base.workers[workerIndex].error ?? undefined,
      };
    } else {
      base.workers.push(payload.worker);
    }
  }

  const hasRunning = base.workers.some((worker) => worker.status === 'running');
  const hasPending = base.workers.some((worker) => worker.status === 'pending');
  const hasFailed = base.workers.some((worker) => worker.status === 'failed');
  if (hasRunning) {
    base.status = 'running';
  } else if (hasPending) {
    base.status = 'pending';
  } else if (hasFailed) {
    base.status = 'failed';
  } else {
    base.status = 'completed';
  }
  base.active_run = base.status === 'running' || base.status === 'pending';
  if (!base.summary?.trim()) {
    base.summary = buildDelegationRuntimeSummary(base);
  }
  return base;
}

function channelLastActivity(channel: ChatChannelSummary) {
  return channel.last_message_at || channel.updated_at || channel.created_at;
}

function normalizeAgentDisplayName(name?: string | null) {
  const raw = (name || '').trim();
  if (!raw) return '';
  const parts = raw
    .split(/\s*-\s*/)
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length <= 1) return raw;
  const deduped: string[] = [];
  for (const part of parts) {
    if (!deduped.includes(part)) {
      deduped.push(part);
    }
  }
  return deduped.join(' - ');
}

function normalizeAgentDisplayNames(names?: Array<string | null | undefined>) {
  const deduped: string[] = [];
  (names || []).forEach((name) => {
    const normalized = normalizeAgentDisplayName(name);
    if (!normalized || deduped.includes(normalized)) {
      return;
    }
    deduped.push(normalized);
  });
  return deduped;
}

function formatDateTime(value?: string | null) {
  if (!value) return '';
  return new Date(value).toLocaleString();
}

function escapeRegExp(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function getMessageMentions(message: ChannelRenderMessage | null | undefined): ChannelMention[] {
  if (!message?.metadata || typeof message.metadata !== 'object') {
    return [];
  }
  const raw = (message.metadata as Record<string, unknown>).mentions;
  if (!Array.isArray(raw)) {
    return [];
  }
  const mentions = raw
    .map((item): ChannelMention | null => {
      if (!item || typeof item !== 'object') {
        return null;
      }
      const mention = item as Record<string, unknown>;
      const mention_type = typeof mention.mention_type === 'string' ? mention.mention_type : '';
      const target_id = typeof mention.target_id === 'string' ? mention.target_id : '';
      const label = typeof mention.label === 'string' ? mention.label : target_id;
      if (!mention_type || !target_id || !label) {
        return null;
      }
      return {
        mention_type,
        target_id,
        label,
      };
    })
    .filter((item): item is ChannelMention => item !== null);
  return mentions;
}

function buildMentionTokenRegex(mentions: Array<Pick<ChannelMention, 'label'>>) {
  const labels = Array.from(
    new Set(
      mentions
        .map((item) => item.label?.trim())
        .filter((value): value is string => Boolean(value)),
    ),
  ).sort((a, b) => b.length - a.length);
  if (labels.length === 0) {
    return null;
  }
  return new RegExp(`@(?:${labels.map(escapeRegExp).join('|')})`, 'g');
}

function hasInlineMentionToken(
  content: string,
  mentions: Array<Pick<ChannelMention, 'label'>>,
) {
  const pattern = buildMentionTokenRegex(mentions);
  if (!pattern) {
    return false;
  }
  return pattern.test(content);
}

function renderMessageContentWithMentions(
  content: string,
  mentions: ChannelMention[],
  tone: 'own' | 'default' = 'default',
): ReactNode {
  const pattern = buildMentionTokenRegex(mentions);
  if (!pattern) {
    return content;
  }
  const nodes: ReactNode[] = [];
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  let segmentIndex = 0;
  while ((match = pattern.exec(content)) !== null) {
    const start = match.index;
    if (start > lastIndex) {
      nodes.push(content.slice(lastIndex, start));
    }
    nodes.push(
      <span
        key={`mention-${segmentIndex}-${start}`}
        className={
          tone === 'own'
            ? 'rounded px-1 py-0.5 font-medium text-primary'
            : 'rounded bg-[hsl(var(--ui-surface-panel-strong))/0.55] px-1 py-0.5 font-medium text-foreground'
        }
      >
        {match[0]}
      </span>,
    );
    lastIndex = start + match[0].length;
    segmentIndex += 1;
  }
  if (lastIndex < content.length) {
    nodes.push(content.slice(lastIndex));
  }
  return nodes;
}

function deriveMentionsFromText(
  text: string,
  options: ComposerMentionOption[],
): ChannelMention[] {
  const normalizedText = text || '';
  const mentions: ChannelMention[] = [];
  const seen = new Set<string>();
  options.forEach((option) => {
    const pattern = new RegExp(`(^|[\\s\\u3000])@${escapeRegExp(option.insertLabel)}(?=$|[\\s\\u3000，。！？、,.!?:;；])`);
    if (!pattern.test(normalizedText)) {
      return;
    }
    const key = `${option.mention_type}:${option.target_id}`;
    if (seen.has(key)) {
      return;
    }
    seen.add(key);
    mentions.push({
      mention_type: option.mention_type,
      target_id: option.target_id,
      label: option.insertLabel,
    });
  });
  return mentions;
}

function findLatestMentionOptionInText(
  text: string,
  options: ComposerMentionOption[],
): ComposerMentionOption | null {
  const normalizedText = text || '';
  let latestOption: ComposerMentionOption | null = null;
  let latestIndex = -1;
  options.forEach((option) => {
    const pattern = new RegExp(
      `@${escapeRegExp(option.insertLabel)}(?=$|[\\s\\u3000，。！？、,.!?:;；])`,
      'g',
    );
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(normalizedText)) !== null) {
      if (match.index >= latestIndex) {
        latestIndex = match.index;
        latestOption = option;
      }
    }
  });
  return latestOption;
}

function detectMentionQuery(text: string, caret: number) {
  const safeCaret = Math.max(0, Math.min(caret, text.length));
  const beforeCaret = text.slice(0, safeCaret);
  const atIndex = beforeCaret.lastIndexOf('@');
  if (atIndex < 0) {
    return null;
  }
  const prefix = atIndex === 0 ? '' : beforeCaret.charAt(atIndex - 1);
  if (prefix && !/\s|[（(【\[{“"'‘]/.test(prefix)) {
    return null;
  }
  const query = beforeCaret.slice(atIndex + 1);
  if (/[\s\r\n]/.test(query)) {
    return null;
  }
  return {
    start: atIndex,
    end: safeCaret,
    query,
  };
}

function summarizeCollaborationSubject(message?: Pick<ChannelRenderMessage, 'content_text' | 'summary_text'> | null) {
  const raw = message?.content_text?.trim() || message?.summary_text?.trim() || '';
  if (!raw) {
    return bilingual('这条协作项', 'This collaboration item');
  }
  const firstLine = raw.split('\n')[0]?.trim() || raw;
  return firstLine.length > 26 ? `${firstLine.slice(0, 26)}…` : firstLine;
}

function formatCompactIdentifier(value?: string | null) {
  const raw = (value || '').trim();
  if (!raw) return '';
  if (raw.length <= 24) return raw;
  return `${raw.slice(0, 8)}-${raw.slice(9, 13)}…${raw.slice(-6)}`;
}

const CHANNEL_AUTONOMY_OPTIONS: Array<{
  mode: ChatChannelAgentAutonomyMode;
  label: string;
  shortLabel: string;
  summary: string;
}> = [
  {
    mode: 'standard',
    label: bilingual('标准模式', 'Standard mode'),
    shortLabel: bilingual('标准模式', 'Standard'),
    summary: bilingual('Agent 主要负责提醒、建议和总结，不主动主导协作推进。', 'The agent focuses on reminders, suggestions, and summaries without actively driving collaboration.'),
  },
  {
    mode: 'proactive',
    label: bilingual('主动推进模式', 'Proactive mode'),
    shortLabel: bilingual('主动推进', 'Proactive'),
    summary: bilingual('Agent 会更积极提醒、建议，并推动协作项启动。', 'The agent proactively reminds, suggests, and helps kick off collaboration items.'),
  },
  {
    mode: 'agent_lead',
    label: bilingual('Agent 主导模式', 'Agent-led mode'),
    shortLabel: bilingual('Agent 主导', 'Agent-led'),
    summary: bilingual('Agent 可以主动创建协作项并推动讨论，但正式发布仍需人工确认。', 'The agent can proactively create collaboration items and drive discussion, but formal publication still requires human confirmation.'),
  },
];

const COLLABORATION_SURFACE_FILTER_OPTIONS: Array<{
  key: CollaborationSurfaceFilter;
  label: string;
  tone: 'neutral' | 'temporary' | 'issue';
}> = [
  { key: 'all', label: bilingual('全部', 'All'), tone: 'neutral' },
  { key: 'temporary', label: bilingual('临时协作', 'Temporary'), tone: 'temporary' },
  { key: 'issue', label: bilingual('正式协作', 'Formal'), tone: 'issue' },
];

const COLLABORATION_STATUS_FILTER_OPTIONS: Array<{
  key: CollaborationStatusFilter;
  label: string;
  tone: 'neutral' | 'idea' | 'progress' | 'decision' | 'success' | 'muted';
}> = [
  { key: 'all', label: bilingual('全部', 'All'), tone: 'neutral' },
  { key: 'proposed', label: bilingual('建议', 'Proposed'), tone: 'idea' },
  { key: 'active', label: bilingual('推进中', 'Active'), tone: 'progress' },
  { key: 'awaiting_confirmation', label: bilingual('等你判断', 'Needs your decision'), tone: 'decision' },
  { key: 'adopted', label: bilingual('已采用', 'Adopted'), tone: 'success' },
  { key: 'rejected', label: bilingual('未采用', 'Rejected'), tone: 'muted' },
];

function getChannelAutonomyMeta(mode?: ChatChannelAgentAutonomyMode | null) {
  return CHANNEL_AUTONOMY_OPTIONS.find((item) => item.mode === mode) || CHANNEL_AUTONOMY_OPTIONS[0];
}

function formatDiscussionDay(value?: string | null) {
  if (!value) return '';
  return new Date(value).toLocaleDateString();
}

function discussionGroupKey(message: ChannelRenderMessage) {
  if (message.author_type === 'agent') {
    return `agent:${message.author_name}:${message.display_kind || 'assistant'}`;
  }
  if (message.author_type === 'system') {
    return `system:${message.display_kind || 'system'}`;
  }
  return `user:${message.author_user_id || message.author_name}:${message.display_kind || 'user'}`;
}

function isGroupedDiscussionMessage(
  previous: ChannelRenderMessage | null,
  current: ChannelRenderMessage,
) {
  if (!previous) return false;
  if (formatDiscussionDay(previous.created_at) !== formatDiscussionDay(current.created_at)) {
    return false;
  }
  if (discussionGroupKey(previous) !== discussionGroupKey(current)) {
    return false;
  }
  const previousTs = new Date(previous.created_at).getTime();
  const currentTs = new Date(current.created_at).getTime();
  return Math.abs(currentTs - previousTs) <= 5 * 60 * 1000;
}

function senderInitial(name?: string | null) {
  const value = (name || '').trim();
  if (!value) return 'A';
  return value.charAt(0).toUpperCase();
}

function DiscussionAvatar({
  kind,
  label,
}: {
  kind: 'user' | 'agent' | 'system';
  label?: string | null;
}) {
  if (kind === 'agent') {
    return (
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-[hsl(var(--ui-line-soft))/0.62] bg-[hsl(var(--background))] text-[hsl(var(--semantic-agent))]">
        <Bot className="h-3.5 w-3.5" />
      </div>
    );
  }
  if (kind === 'system') {
    return (
      <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-[hsl(var(--ui-line-soft))/0.62] bg-[hsl(var(--background))] text-muted-foreground">
        <ClipboardList className="h-3.5 w-3.5" />
      </div>
    );
  }
  return (
    <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-[hsl(var(--ui-line-soft))/0.62] bg-[hsl(var(--background))] text-[10px] font-semibold text-foreground">
      {senderInitial(label)}
    </div>
  );
}

function fromChannelMessage(message: ChatChannelMessage): ChannelRenderMessage {
  return { ...message };
}

function collaborationStatusLabel(
  status?: ChatChannelDisplayStatus | null,
  fallbackSurface?: ChatChannelMessageSurface,
): string {
  if (status === 'proposed') return bilingual('建议', 'Proposed');
  if (status === 'awaiting_confirmation') return bilingual('等你判断', 'Needs your decision');
  if (status === 'adopted') return bilingual('已采用', 'Adopted');
  if (status === 'rejected') return bilingual('未采用', 'Rejected');
  if (status === 'active') return bilingual('推进中', 'Active');
  if (fallbackSurface === 'activity') return bilingual('讨论', 'Discussion');
  return bilingual('推进中', 'Active');
}

function collaborationStatusTone(
  status?: ChatChannelDisplayStatus | null,
  _fallbackSurface?: ChatChannelMessageSurface,
): string {
  if (status === 'adopted') {
    return 'bg-emerald-500/10 text-emerald-800';
  }
  if (status === 'rejected') {
    return 'bg-rose-500/10 text-rose-800';
  }
  if (status === 'awaiting_confirmation') {
    return 'bg-sky-500/10 text-sky-800';
  }
  if (status === 'proposed') {
    return 'bg-violet-500/10 text-violet-800';
  }
  return 'bg-amber-500/10 text-amber-800';
}

function collaborationSurfaceLabel(surface?: ChatChannelMessageSurface | null): string {
  if (surface === 'temporary') return bilingual('临时协作', 'Temporary');
  if (surface === 'issue') return bilingual('正式协作', 'Formal');
  if (surface === 'activity') return bilingual('讨论', 'Discussion');
  return bilingual('协作', 'Collaboration');
}

function collaborationSurfaceTone(surface?: ChatChannelMessageSurface | null): string {
  if (surface === 'temporary') {
    return 'bg-sky-500/10 text-sky-800';
  }
  if (surface === 'issue') {
    return 'bg-amber-500/10 text-amber-800';
  }
  return 'bg-muted/60 text-muted-foreground';
}

function collaborationSurfaceFilterLabel(filter: CollaborationSurfaceFilter): string {
  if (filter === 'temporary') return bilingual('临时协作', 'Temporary');
  if (filter === 'issue') return bilingual('正式协作', 'Formal');
  return bilingual('全部协作', 'All collaboration');
}

function collaborationSurfaceHint(surface?: ChatChannelMessageSurface | null): string {
  if (surface === 'temporary') {
    return bilingual('先聊明白，确认后升级为正式协作', 'Clarify the discussion first, then promote it to formal collaboration.');
  }
  if (surface === 'issue') {
    return bilingual('已进入正式推进，静默一段时间后会同步阶段进展到讨论区', 'This item is now in formal execution. Stage updates will sync back to the discussion area after a quiet period.');
  }
  return '';
}

function collaborationStatusFilterLabel(filter: CollaborationStatusFilter): string {
  if (filter === 'proposed') return bilingual('建议', 'Proposed');
  if (filter === 'active') return bilingual('推进中', 'Active');
  if (filter === 'awaiting_confirmation') return bilingual('等你判断', 'Needs your decision');
  if (filter === 'adopted') return bilingual('已采用', 'Adopted');
  if (filter === 'rejected') return bilingual('未采用', 'Rejected');
  return bilingual('全部', 'All');
}

function collaborationWorklistTitle(
  surfaceFilter: CollaborationSurfaceFilter,
  statusFilter: CollaborationStatusFilter,
): string {
  if (surfaceFilter === 'all' && statusFilter === 'all') {
    return bilingual('还没有协作项', 'No collaboration items yet');
  }
  if (surfaceFilter === 'all') {
    return bilingual(`还没有${collaborationStatusFilterLabel(statusFilter)}的协作项`, `No ${collaborationStatusFilterLabel(statusFilter).toLowerCase()} collaboration items yet`);
  }
  if (statusFilter === 'all') {
    return bilingual(`还没有${collaborationSurfaceFilterLabel(surfaceFilter)}`, `No ${collaborationSurfaceFilterLabel(surfaceFilter).toLowerCase()} items yet`);
  }
  return bilingual(`还没有${collaborationStatusFilterLabel(statusFilter)}的${collaborationSurfaceFilterLabel(surfaceFilter)}`, `No ${collaborationStatusFilterLabel(statusFilter).toLowerCase()} ${collaborationSurfaceFilterLabel(surfaceFilter).toLowerCase()} items yet`);
}

function collaborationWorklistDescription(
  surfaceFilter: CollaborationSurfaceFilter,
  statusFilter: CollaborationStatusFilter,
): string {
  if (surfaceFilter === 'temporary' && statusFilter === 'all') {
    return bilingual('这里集中显示先聊明白、补充上下文和试探方向的临时协作。它们还没有被提炼成正式协作项。', 'Temporary collaboration lives here for clarifying context and exploring direction before becoming a formal item.');
  }
  if (surfaceFilter === 'issue' && statusFilter === 'all') {
    return bilingual('这里集中显示已经进入正式推进的协作项，适合查看重点工作、结果和后续判断。', 'Formal collaboration lives here for tracking important work, results, and next decisions.');
  }
  if (surfaceFilter === 'all' && statusFilter === 'all') {
    return bilingual('先在讨论模式把事情说清楚，或者直接在下面新建一条协作项。', 'Clarify the work in discussion mode first, or create a new collaboration item directly below.');
  }
  if (surfaceFilter === 'all') {
    return bilingual('换个状态筛选看看，或者直接新建一条新的协作项。', 'Try another status filter, or create a new collaboration item directly.');
  }
  return bilingual(`当前筛选的是${collaborationSurfaceFilterLabel(surfaceFilter)}，可以换个状态看看，或者先回到全部协作。`, `You are filtering ${collaborationSurfaceFilterLabel(surfaceFilter).toLowerCase()} items. Try another status, or switch back to all collaboration.`);
}

function assistantCardMeta(message: ChannelRenderMessage): {
  label: string;
  icon: LucideIcon;
  bubbleClass: string;
  chipClass: string;
  iconClass: string;
} {
  const cardPurpose = message.metadata?.card_purpose as string | undefined;
  if (cardPurpose === 'discussion_summary') {
    return {
    label: bilingual('总结卡', 'Summary card'),
      icon: ClipboardList,
      bubbleClass: 'bg-[hsl(var(--ui-surface-panel-strong))/0.52]',
      chipClass: 'bg-[hsl(var(--ui-surface-panel-strong))/0.65] text-muted-foreground',
      iconClass: 'bg-muted/50 text-muted-foreground',
    };
  }
  if (
    cardPurpose === 'collaboration_awaiting_judgement_reminder' ||
    cardPurpose === 'collaboration_stalled_reminder'
  ) {
    return {
    label: bilingual('提醒卡', 'Reminder card'),
      icon: TriangleAlert,
      bubbleClass: 'bg-amber-500/8',
      chipClass: 'bg-amber-500/10 text-amber-800',
      iconClass: 'bg-amber-100/60 text-amber-800',
    };
  }
  if (cardPurpose === 'formal_collaboration_progress_sync') {
    return {
    label: bilingual('进度卡', 'Progress card'),
      icon: MessageSquareReply,
      bubbleClass: 'bg-indigo-500/8',
      chipClass: 'bg-indigo-500/10 text-indigo-800',
      iconClass: 'bg-indigo-100/60 text-indigo-800',
    };
  }
  if (message.display_kind === 'onboarding') {
    return {
    label: bilingual('启动卡', 'Start card'),
      icon: ClipboardList,
      bubbleClass: 'bg-violet-500/8',
      chipClass: 'bg-violet-500/10 text-violet-800',
      iconClass: 'bg-violet-100/60 text-violet-800',
    };
  }
  if (message.display_kind === 'suggestion') {
    return {
    label: bilingual('建议卡', 'Suggestion card'),
      icon: Lightbulb,
      bubbleClass: 'bg-sky-500/8',
      chipClass: 'bg-sky-500/10 text-sky-800',
      iconClass: 'bg-sky-100/60 text-sky-800',
    };
  }
  if (message.display_kind === 'result') {
    return {
    label: bilingual('结果卡', 'Result card'),
      icon: CheckCheck,
      bubbleClass: 'bg-emerald-500/8',
      chipClass: 'bg-emerald-500/10 text-emerald-800',
      iconClass: 'bg-emerald-100/60 text-emerald-800',
    };
  }
  return {
    label: bilingual('AI 回答', 'AI reply'),
    icon: Bot,
      bubbleClass: 'bg-[hsl(var(--ui-surface-panel-strong))/0.48]',
      chipClass: 'bg-[hsl(var(--ui-surface-panel-strong))/0.62] text-muted-foreground',
      iconClass: 'bg-muted/50 text-muted-foreground',
    };
  }

const workStatusLabel = collaborationStatusLabel;
const workStatusTone = collaborationStatusTone;

function getAttachedDocumentIds(message: ChannelRenderMessage | null | undefined): string[] {
  if (!message?.metadata || typeof message.metadata !== 'object') {
    return [];
  }
  const raw = (message.metadata as Record<string, unknown>).attached_document_ids;
  if (!Array.isArray(raw)) {
    return [];
  }
  return raw.filter((item): item is string => typeof item === 'string' && item.trim().length > 0);
}

interface CodingCardTests {
  status?: string | null;
  summary?: string | null;
  passed?: number | null;
  failed?: number | null;
  skipped?: number | null;
  last_command?: string | null;
}

interface CodingCardPayload {
  workspace_display_name?: string | null;
  workspace_path?: string | null;
  repo_path?: string | null;
  main_checkout_path?: string | null;
  repo_default_branch?: string | null;
  thread_worktree_path?: string | null;
  thread_branch?: string | null;
  thread_repo_ref?: string | null;
  changed_files: string[];
  tests?: CodingCardTests | null;
  commands_run: string[];
  artifacts: string[];
  blockers: string[];
  next_action?: string | null;
}

function getCodingCardPayload(
  message: ChannelRenderMessage | null | undefined,
): CodingCardPayload | null {
  if (!message?.metadata || typeof message.metadata !== 'object') {
    return null;
  }
  const metadata = message.metadata as Record<string, unknown>;
  if (metadata.card_domain !== 'coding') {
    return null;
  }
  const raw = metadata.coding_payload;
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    return null;
  }
  const value = raw as Record<string, unknown>;
  const asStringArray = (input: unknown) =>
    Array.isArray(input)
      ? input.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
      : [];
  const testsRaw =
    value.tests && typeof value.tests === 'object' && !Array.isArray(value.tests)
      ? (value.tests as Record<string, unknown>)
      : null;
  return {
    workspace_display_name:
      typeof value.workspace_display_name === 'string' ? value.workspace_display_name : null,
    workspace_path: typeof value.workspace_path === 'string' ? value.workspace_path : null,
    repo_path: typeof value.repo_path === 'string' ? value.repo_path : null,
    main_checkout_path:
      typeof value.main_checkout_path === 'string' ? value.main_checkout_path : null,
    repo_default_branch:
      typeof value.repo_default_branch === 'string' ? value.repo_default_branch : null,
    thread_worktree_path:
      typeof value.thread_worktree_path === 'string' ? value.thread_worktree_path : null,
    thread_branch: typeof value.thread_branch === 'string' ? value.thread_branch : null,
    thread_repo_ref: typeof value.thread_repo_ref === 'string' ? value.thread_repo_ref : null,
    changed_files: asStringArray(value.changed_files),
    tests: testsRaw
      ? {
          status: typeof testsRaw.status === 'string' ? testsRaw.status : null,
          summary: typeof testsRaw.summary === 'string' ? testsRaw.summary : null,
          passed: typeof testsRaw.passed === 'number' ? testsRaw.passed : null,
          failed: typeof testsRaw.failed === 'number' ? testsRaw.failed : null,
          skipped: typeof testsRaw.skipped === 'number' ? testsRaw.skipped : null,
          last_command:
            typeof testsRaw.last_command === 'string' ? testsRaw.last_command : null,
        }
      : null,
    commands_run: asStringArray(value.commands_run),
    artifacts: asStringArray(value.artifacts),
    blockers: asStringArray(value.blockers),
    next_action: typeof value.next_action === 'string' ? value.next_action : null,
  };
}

function CodingCardDetails({ payload }: { payload: CodingCardPayload }) {
  const changedFiles = payload.changed_files.slice(0, 8);
  const commandsRun = payload.commands_run.slice(0, 4);
  const artifacts = payload.artifacts.slice(0, 4);
  const blockers = payload.blockers.slice(0, 3);
  return (
    <div className="mt-2 rounded-[12px] border border-[hsl(var(--ui-line-soft))/0.55] bg-background/60 px-3 py-2 text-[11px] text-muted-foreground">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium text-foreground">
          {payload.workspace_display_name || bilingual('项目工作区', 'Project workspace')}
        </span>
        {payload.thread_branch ? (
          <span className="rounded-full bg-muted/60 px-2 py-0.5 text-[10px]">
            {bilingual('分支：', 'Branch: ')}{payload.thread_branch}
          </span>
        ) : null}
        {payload.repo_default_branch ? (
          <span className="rounded-full bg-muted/60 px-2 py-0.5 text-[10px]">
            {bilingual('默认分支：', 'Default branch: ')}{payload.repo_default_branch}
          </span>
        ) : null}
      </div>
      {payload.thread_worktree_path ? (
        <div className="mt-1.5 break-all">{bilingual('线程现场：', 'Thread workspace: ')}{payload.thread_worktree_path}</div>
      ) : payload.workspace_path ? (
        <div className="mt-1.5 break-all">{bilingual('工作区：', 'Workspace: ')}{payload.workspace_path}</div>
      ) : null}
      {payload.repo_path ? <div className="mt-1 break-all">{bilingual('仓库：', 'Repo: ')}{payload.repo_path}</div> : null}
      {payload.main_checkout_path ? (
        <div className="mt-1 break-all">{bilingual('主检出：', 'Main checkout: ')}{payload.main_checkout_path}</div>
      ) : null}
      {changedFiles.length > 0 ? (
        <div className="mt-2">
          <div className="font-medium text-foreground">{bilingual('变更文件', 'Changed files')}</div>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {changedFiles.map((file) => (
              <span key={file} className="rounded-full bg-muted/60 px-2 py-0.5 text-[10px]">
                {file}
              </span>
            ))}
          </div>
        </div>
      ) : null}
      {payload.tests && (payload.tests.status || payload.tests.summary) ? (
        <div className="mt-2">
          <div className="font-medium text-foreground">{bilingual('测试', 'Tests')}</div>
          <div className="mt-1">
            {payload.tests.status ? `${bilingual('状态：', 'Status: ')}${payload.tests.status}` : ''}
            {payload.tests.status && payload.tests.summary ? ' · ' : ''}
            {payload.tests.summary || ''}
          </div>
        </div>
      ) : null}
      {commandsRun.length > 0 ? (
        <div className="mt-2">
          <div className="font-medium text-foreground">{bilingual('最近命令', 'Recent commands')}</div>
          <ul className="mt-1 space-y-1">
            {commandsRun.map((command) => (
              <li key={command} className="break-all">
                {command}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {artifacts.length > 0 ? (
        <div className="mt-2">
          <div className="font-medium text-foreground">{bilingual('产物', 'Artifacts')}</div>
          <div className="mt-1 flex flex-wrap gap-1.5">
            {artifacts.map((artifact) => (
              <span key={artifact} className="rounded-full bg-muted/60 px-2 py-0.5 text-[10px]">
                {artifact}
              </span>
            ))}
          </div>
        </div>
      ) : null}
      {blockers.length > 0 ? (
        <div className="mt-2">
          <div className="font-medium text-foreground">{bilingual('当前阻塞', 'Current blockers')}</div>
          <ul className="mt-1 space-y-1">
            {blockers.map((blocker) => (
              <li key={blocker}>{blocker}</li>
            ))}
          </ul>
        </div>
      ) : null}
      {payload.next_action ? (
        <div className="mt-2">
          <div className="font-medium text-foreground">{bilingual('下一步', 'Next steps')}</div>
          <div className="mt-1">{payload.next_action}</div>
        </div>
      ) : null}
    </div>
  );
}

interface RuntimeDiagnosticsMetadata {
  status?: string;
  summary?: string;
  validation_status?: string | null;
  blocking_reason?: string | null;
  next_steps?: string[];
  reason_code?: string | null;
  content_accessed?: boolean | null;
  analysis_complete?: boolean | null;
}

function getRuntimeDiagnostics(
  message: ChannelRenderMessage | null | undefined,
): RuntimeDiagnosticsMetadata | null {
  if (!message?.metadata || typeof message.metadata !== 'object') {
    return null;
  }
  const raw = (message.metadata as Record<string, unknown>).runtime_diagnostics;
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
    return null;
  }
  const value = raw as Record<string, unknown>;
  const nextSteps = Array.isArray(value.next_steps)
    ? value.next_steps.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
    : [];
  return {
    status: typeof value.status === 'string' ? value.status : undefined,
    summary: typeof value.summary === 'string' ? value.summary : undefined,
    validation_status:
      typeof value.validation_status === 'string' ? value.validation_status : null,
    blocking_reason:
      typeof value.blocking_reason === 'string' ? value.blocking_reason : null,
    next_steps: nextSteps,
    reason_code: typeof value.reason_code === 'string' ? value.reason_code : null,
    content_accessed:
      typeof value.content_accessed === 'boolean' ? value.content_accessed : null,
    analysis_complete:
      typeof value.analysis_complete === 'boolean' ? value.analysis_complete : null,
  };
}

function runtimeDiagnosticStatusLabel(status?: string | null) {
  if (status === 'completed') return bilingual('已完成', 'Completed');
  if (status === 'blocked') return bilingual('阻塞', 'Blocked');
  if (status === 'failed') return bilingual('失败', 'Failed');
  return status || bilingual('执行详情', 'Execution details');
}

function runtimeDiagnosticFriendlySummary(diagnostics: RuntimeDiagnosticsMetadata) {
  if (diagnostics.status === 'completed') {
    return bilingual('系统已记录这轮执行情况，可按需查看详细处理记录。', 'This execution was recorded successfully. Open the detail view if you need the full trace.');
  }
  return bilingual('系统记录到这轮协作未完整结束。可以补充背景、资料或更明确的目标后再试。', 'This collaboration did not finish cleanly. Add more context, documents, or a clearer goal and try again.');
}

function buildOptimisticUserMessage(
  channelId: string,
  user: TeamMember | null,
  content: string,
  surface: ChatChannelMessageSurface,
  mentions: ChannelMention[] = [],
  threadRootId?: string | null,
  parentMessageId?: string | null,
): ChannelRenderMessage {
  const now = new Date().toISOString();
  return {
    message_id: `optimistic-user-${Date.now()}`,
    channel_id: channelId,
    team_id: '',
    author_type: 'user',
    author_user_id: user?.userId || null,
      author_name: user?.displayName || bilingual('我', 'Me'),
    agent_id: null,
    surface,
    thread_state: 'active',
    display_kind: surface === 'activity' ? 'discussion' : 'collaboration',
    display_status: surface === 'activity' ? null : 'active',
    source_kind: 'human',
    has_ai_participation: surface !== 'activity',
    summary_text: content,
    recent_agent_names: [],
    content_text: content,
    content_blocks: [],
    metadata: mentions.length > 0 ? { mentions } : {},
    visible: true,
    created_at: now,
    updated_at: now,
    reply_count: 0,
    thread_root_id: threadRootId || null,
    parent_message_id: parentMessageId || null,
  };
}

function buildStreamingAssistantMessage(
  channelId: string,
  agentName: string,
  surface: ChatChannelMessageSurface,
  threadRootId?: string | null,
  parentMessageId?: string | null,
): ChannelRenderMessage {
  const now = new Date().toISOString();
  return {
    message_id: `streaming-assistant-${Date.now()}`,
    channel_id: channelId,
    team_id: '',
    author_type: 'agent',
    author_agent_id: null,
    author_name: agentName,
    agent_id: null,
    surface,
    thread_state: 'active',
    display_kind: 'collaboration',
    display_status: 'active',
    source_kind: 'agent',
    has_ai_participation: true,
    summary_text: '',
    recent_agent_names: [agentName],
    content_text: '',
    content_blocks: [],
    metadata: {},
    visible: true,
    created_at: now,
    updated_at: now,
    reply_count: 0,
    thread_root_id: threadRootId || null,
    parent_message_id: parentMessageId || null,
    isStreaming: true,
    toolCalls: [],
  };
}

function ThreadSummaryRow({
  replyCount,
  documentCount,
  aiOutputCount,
  previewText,
  onOpenThread,
  surface = 'issue',
  status,
  onPromote,
  onArchive,
  align = 'start',
}: {
  replyCount: number;
  documentCount: number;
  aiOutputCount: number;
  previewText?: string | null;
  onOpenThread: () => void;
  surface?: ChatChannelMessageSurface;
  status?: ChatChannelDisplayStatus | null;
  onPromote?: (() => void) | null;
  onArchive?: (() => void) | null;
  align?: 'start' | 'end';
}) {
  const compactPreview = previewText?.trim()
    ? (previewText.trim().length > 72 ? `${previewText.trim().slice(0, 72)}…` : previewText.trim())
    : null;
  return (
    <div className={`mt-1.5 flex ${align === 'end' ? 'justify-end' : 'justify-start'}`}>
      <div className="inline-flex max-w-[720px] flex-col gap-1">
        <button
          type="button"
          onClick={onOpenThread}
          className="inline-flex min-w-0 items-center gap-2 rounded-[10px] border border-[hsl(var(--ui-line-soft))/0.58] bg-muted/[0.04] px-2.5 py-1.5 text-left transition-colors hover:bg-accent/20"
        >
          <span className="inline-flex shrink-0 items-center gap-1 text-[10px] font-medium text-foreground">
            <MessageSquareReply className="h-3.5 w-3.5 text-muted-foreground" />
            {bilingual('协作', 'Collaboration')}
          </span>
          <span className={`inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-[10px] ${
            collaborationSurfaceTone(surface)
          }`}>
            {collaborationSurfaceLabel(surface)}
          </span>
          {status ? (
            <span className={`inline-flex shrink-0 items-center rounded-full px-2 py-0.5 text-[10px] ${
              workStatusTone(status, surface)
            }`}>
              {workStatusLabel(status, surface)}
            </span>
          ) : null}
          <span className="shrink-0 text-[10px] text-muted-foreground">
            {bilingual(`${replyCount} 条回复`, `${replyCount} replies`)}
          </span>
          {documentCount > 0 ? (
            <span className="shrink-0 text-[10px] text-muted-foreground">
               {bilingual(`${documentCount} 资料`, `${documentCount} docs`)}
            </span>
          ) : null}
          {aiOutputCount > 0 ? (
            <span className="shrink-0 text-[10px] text-primary">
               {bilingual(`${aiOutputCount} 个 AI 产出`, `${aiOutputCount} AI outputs`)}
            </span>
          ) : null}
          <span className="min-w-0 flex-1 truncate text-[10px] text-muted-foreground">
            {compactPreview || bilingual('打开线程查看完整回复', 'Open the thread to view the full reply')}
          </span>
          <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </button>
        {collaborationSurfaceHint(surface) ? (
          <div className="flex flex-col gap-1 px-1">
            <div className="collab-surface-hint">
              {collaborationSurfaceHint(surface)}
            </div>
            {surface === 'temporary' && (onPromote || onArchive) ? (
              <div className="flex flex-wrap items-center gap-1.5">
                {onPromote ? (
                  <button
                    type="button"
                    onClick={onPromote}
                    className="inline-flex items-center rounded-full bg-primary/[0.08] px-2 py-0.5 text-[10px] font-medium text-primary"
                  >
                    {bilingual('升级为正式协作', 'Promote to formal collaboration')}
                  </button>
                ) : null}
                {onArchive ? (
                  <button
                    type="button"
                    onClick={onArchive}
                    className="inline-flex items-center rounded-full bg-background/70 px-2 py-0.5 text-[10px] font-medium text-muted-foreground"
                  >
                    <Archive className="mr-1 h-3 w-3" />
                    {bilingual('标记未采用', 'Mark as rejected')}
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function ChannelActivityBubble({
  message,
  isOwn,
  groupedWithPrevious = false,
}: {
  message: ChannelRenderMessage;
  isOwn: boolean;
  groupedWithPrevious?: boolean;
}) {
  const mentions = getMessageMentions(message);
  const showMentionFallback = mentions.length > 0 && !hasInlineMentionToken(message.content_text, mentions);
  return (
    <div className={`flex w-full gap-2.5 ${isOwn ? 'flex-row-reverse' : 'flex-row'} items-start ${groupedWithPrevious ? 'mt-1' : 'mt-3'}`}>
      {groupedWithPrevious ? (
        <div className="h-7 w-7 shrink-0" />
      ) : (
        <DiscussionAvatar kind="user" label={isOwn ? bilingual('你', 'You') : message.author_name} />
      )}
      <div className={`flex min-w-0 max-w-[96%] flex-col ${isOwn ? 'items-end md:max-w-[92%] xl:max-w-[82%]' : 'items-start md:max-w-[92%] xl:max-w-[84%]'}`}>
        {!groupedWithPrevious ? (
          <div className="mb-1 flex items-center gap-1.5 px-1 text-[11px] text-muted-foreground">
            <span className="font-medium text-foreground">{isOwn ? bilingual('你', 'You') : message.author_name}</span>
            <span>{formatDateTime(message.created_at)}</span>
          </div>
        ) : null}
        <div className={`w-full rounded-[14px] px-3 py-2.5 ${
          isOwn
            ? 'bg-[hsl(var(--primary))/0.06] text-foreground'
            : 'bg-[hsl(var(--ui-surface-panel-strong))/0.24]'
        }`}>
          {showMentionFallback ? (
            <div className={`mb-1.5 flex flex-wrap items-center gap-1.5 text-[11px] ${
              isOwn ? 'text-primary' : 'text-muted-foreground'
            }`}>
              {mentions.map((mention) => (
                <span key={`${mention.mention_type}:${mention.target_id}`} className={`rounded-full px-2 py-0.5 ${
                  isOwn ? 'bg-[hsl(var(--primary))/0.08] text-primary' : 'bg-muted/45'
                }`}>
                  @{mention.label}
                </span>
              ))}
            </div>
          ) : null}
          <div className="whitespace-pre-wrap break-words text-[13px] leading-6 text-foreground">
            {renderMessageContentWithMentions(message.content_text, mentions, isOwn ? 'own' : 'default')}
          </div>
        </div>
      </div>
    </div>
  );
}

function ChannelUserBubble({
  message,
  isOwn,
  threadDocumentCount,
  threadAiOutputCount,
  threadPreviewText,
  onOpenThread,
  surface,
  showThreadSummary,
  onPromote,
  onArchive,
  groupedWithPrevious = false,
}: {
  message: ChannelRenderMessage;
  isOwn: boolean;
  threadDocumentCount: number;
  threadAiOutputCount: number;
  threadPreviewText?: string | null;
  onOpenThread: (message: ChannelRenderMessage) => void;
  surface: ChatChannelMessageSurface;
  showThreadSummary: boolean;
  onPromote?: (() => void) | null;
  onArchive?: (() => void) | null;
  groupedWithPrevious?: boolean;
}) {
  const mentions = getMessageMentions(message);
  const showMentionFallback = mentions.length > 0 && !hasInlineMentionToken(message.content_text, mentions);
  return (
    <div className={`flex w-full gap-2.5 ${isOwn ? 'flex-row-reverse' : 'flex-row'} items-start ${groupedWithPrevious ? 'mt-1' : 'mt-3'}`}>
      {groupedWithPrevious ? (
        <div className="h-7 w-7 shrink-0" />
      ) : (
      <DiscussionAvatar kind="user" label={isOwn ? bilingual('你', 'You') : message.author_name} />
      )}
      <div className={`flex min-w-0 max-w-[96%] flex-col ${isOwn ? 'items-end md:max-w-[92%] xl:max-w-[82%]' : 'items-start md:max-w-[92%] xl:max-w-[84%]'}`}>
        {!groupedWithPrevious ? (
          <div className="mb-1 flex items-center gap-1.5 px-1 text-[11px] text-muted-foreground">
            <span className="font-medium text-foreground">{isOwn ? bilingual('你', 'You') : message.author_name}</span>
            <span>{formatDateTime(message.created_at)}</span>
          </div>
        ) : null}
        <div className={`w-full rounded-[14px] px-3 py-2.5 ${
          isOwn
            ? 'bg-[hsl(var(--primary))/0.06] text-foreground'
            : 'bg-[hsl(var(--ui-surface-panel-strong))/0.26]'
        }`}>
          {showMentionFallback ? (
            <div className={`mb-1.5 flex flex-wrap items-center gap-1.5 text-[11px] ${
              isOwn ? 'text-primary' : 'text-muted-foreground'
            }`}>
              {mentions.map((mention) => (
                <span key={`${mention.mention_type}:${mention.target_id}`} className={`rounded-full px-2 py-0.5 ${
                  isOwn ? 'bg-[hsl(var(--primary))/0.08] text-primary' : 'bg-muted/45'
                }`}>
                  @{mention.label}
                </span>
              ))}
            </div>
          ) : null}
          <div className={`text-[11px] font-medium uppercase tracking-[0.04em] ${isOwn ? 'text-primary' : 'text-muted-foreground'}`}>
            {bilingual('发起内容', 'Original request')}
          </div>
          <div className="mt-1 line-clamp-3 whitespace-pre-wrap break-words text-[13px] leading-6 text-foreground">
            {renderMessageContentWithMentions(message.content_text, mentions, isOwn ? 'own' : 'default')}
          </div>
        </div>
        {showThreadSummary ? (
          <ThreadSummaryRow
            replyCount={message.reply_count}
            documentCount={threadDocumentCount}
            aiOutputCount={threadAiOutputCount}
            previewText={threadPreviewText}
            surface={surface}
            status={message.display_status}
            onOpenThread={() => onOpenThread(message)}
            onPromote={onPromote}
            onArchive={onArchive}
            align={isOwn ? 'end' : 'start'}
          />
        ) : null}
      </div>
    </div>
  );
}

function ChannelAssistantBubble({
  message,
  threadDocumentCount,
  threadAiOutputCount,
  threadPreviewText,
  onOpenThread,
  surface,
  showThreadSummary,
  onPromote,
  onArchive,
  primaryActionLabel,
  onPrimaryAction,
  secondaryActionLabel,
  onSecondaryAction,
  groupedWithPrevious = false,
}: {
  message: ChannelRenderMessage;
  threadDocumentCount: number;
  threadAiOutputCount: number;
  threadPreviewText?: string | null;
  onOpenThread: (message: ChannelRenderMessage) => void;
  surface: ChatChannelMessageSurface;
  showThreadSummary: boolean;
  onPromote?: (() => void) | null;
  onArchive?: (() => void) | null;
  primaryActionLabel?: string | null;
  onPrimaryAction?: (() => void) | null;
  secondaryActionLabel?: string | null;
  onSecondaryAction?: (() => void) | null;
  groupedWithPrevious?: boolean;
}) {
  const cardMeta = assistantCardMeta(message);
  const CardIcon = cardMeta.icon;
  const codingPayload = getCodingCardPayload(message);

  return (
    <div className={`flex w-full items-start gap-2.5 ${groupedWithPrevious ? 'mt-1' : 'mt-3'}`}>
      {groupedWithPrevious ? (
        <div className="h-7 w-7 shrink-0" />
      ) : (
        <DiscussionAvatar kind="agent" />
      )}
      <div className="flex min-w-0 max-w-[96%] flex-col items-start md:max-w-[92%] xl:max-w-[86%]">
        {!groupedWithPrevious ? (
          <div className="mb-1 flex items-center gap-1.5 px-1 text-[10px] text-muted-foreground">
            <span className={`inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${cardMeta.chipClass}`}>
              <CardIcon className="h-3 w-3" />
              {cardMeta.label}
            </span>
            <span className="font-medium text-foreground">
              {normalizeAgentDisplayName(message.author_name)}
            </span>
            <span>{formatDateTime(message.created_at)}</span>
          </div>
        ) : null}
        <div className={`w-full rounded-[14px] px-3 py-2.5 shadow-none ${cardMeta.bubbleClass}`}>
          <div className="whitespace-pre-wrap break-words text-[12px] leading-5 text-foreground">
            {message.content_text}
          </div>
          {codingPayload ? <CodingCardDetails payload={codingPayload} /> : null}
          {((onPrimaryAction && primaryActionLabel) || (onSecondaryAction && secondaryActionLabel)) ? (
            <div className="mt-2 flex flex-wrap items-center gap-1.5">
              {onPrimaryAction && primaryActionLabel ? (
                <button
                  type="button"
                  onClick={onPrimaryAction}
                  className={`inline-flex items-center gap-1 rounded-full px-2.5 py-1 text-[10px] font-medium transition-colors ${cardMeta.chipClass}`}
                >
                  <Sparkles className="h-3 w-3" />
                  {primaryActionLabel}
                </button>
              ) : null}
              {onSecondaryAction && secondaryActionLabel ? (
                <button
                  type="button"
                  onClick={onSecondaryAction}
                  className="inline-flex items-center gap-1 rounded-full bg-[hsl(var(--ui-surface-panel-strong))/0.42] px-2.5 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:text-foreground"
                >
                  <X className="h-3 w-3" />
                  {secondaryActionLabel}
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
        {!message.isStreaming && showThreadSummary ? (
          <ThreadSummaryRow
            replyCount={message.reply_count}
            documentCount={threadDocumentCount}
            aiOutputCount={threadAiOutputCount}
            previewText={threadPreviewText}
            surface={surface}
            status={message.display_status}
            onOpenThread={() => onOpenThread(message)}
            onPromote={onPromote}
            onArchive={onArchive}
          />
        ) : null}
      </div>
    </div>
  );
}

function RuntimeDiagnosticDisclosure({
  diagnostics,
  showFull,
}: {
  diagnostics: RuntimeDiagnosticsMetadata;
  showFull: boolean;
}) {
  if (!showFull) {
    return (
      <details className="collab-diagnostic-details">
        <summary>
                  <span>{bilingual('处理说明', 'Handling notes')}</span>
          {diagnostics.status ? (
            <span className="collab-diagnostic-status">
              {runtimeDiagnosticStatusLabel(diagnostics.status)}
            </span>
          ) : null}
        </summary>
        <div className="collab-diagnostic-body">
          <div className="collab-diagnostic-friendly">
            {runtimeDiagnosticFriendlySummary(diagnostics)}
          </div>
        </div>
      </details>
    );
  }

  return (
    <details className="collab-diagnostic-details">
      <summary>
                  <span>{bilingual('执行详情', 'Execution details')}</span>
        {diagnostics.status ? (
          <span className="collab-diagnostic-status">
            {runtimeDiagnosticStatusLabel(diagnostics.status)}
          </span>
        ) : null}
      </summary>
      <div className="collab-diagnostic-body">
        {diagnostics.summary ? (
          <div className="collab-diagnostic-line">
                  <span className="collab-diagnostic-label">{bilingual('摘要', 'Summary')}</span>
            <span>{diagnostics.summary}</span>
          </div>
        ) : null}
        {diagnostics.validation_status ? (
          <div className="collab-diagnostic-line">
            <span className="collab-diagnostic-label">Validation</span>
            <span>{diagnostics.validation_status}</span>
          </div>
        ) : null}
        {diagnostics.blocking_reason ? (
          <div className="collab-diagnostic-line">
            <span className="collab-diagnostic-label">Blocking</span>
            <span>{diagnostics.blocking_reason}</span>
          </div>
        ) : null}
        {diagnostics.next_steps && diagnostics.next_steps.length > 0 ? (
          <div className="collab-diagnostic-line">
            <span className="collab-diagnostic-label">Next steps</span>
            <ul className="collab-diagnostic-list">
              {diagnostics.next_steps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
    </details>
  );
}

function ChannelSystemBubble({
  message,
  showFullDiagnostics = false,
}: {
  message: ChannelRenderMessage;
  showFullDiagnostics?: boolean;
}) {
  const diagnostics = getRuntimeDiagnostics(message);
  return (
    <div className="mt-3 flex justify-center">
      {diagnostics ? (
        <div className="collab-diagnostic-note">
          <div className="collab-diagnostic-summary">
            {message.content_text}
          </div>
          <RuntimeDiagnosticDisclosure diagnostics={diagnostics} showFull={showFullDiagnostics} />
        </div>
      ) : (
        <div className="rounded-full bg-[hsl(var(--ui-surface-panel-strong))/0.38] px-2.5 py-1 text-[10px] text-muted-foreground">
          {message.content_text}
        </div>
      )}
    </div>
  );
}

function DiscussionDayDivider({ label }: { label: string }) {
  return (
    <div className="flex items-center justify-center py-3">
      <div className="collab-divider-label collab-micro">
        {label}
      </div>
    </div>
  );
}

function ThreadStatusBar({
  replyCount,
  documentCount,
  aiOutputCount,
  compact = false,
}: {
  replyCount: number;
  documentCount: number;
  aiOutputCount: number;
  compact?: boolean;
}) {
  return (
    <div className={compact ? 'collab-thread-stats-row' : 'flex flex-wrap items-center gap-1.5'}>
      <span className="collab-thread-stat" data-compact={compact ? 'true' : undefined}>
            {bilingual(`${replyCount} 条回复`, `${replyCount} replies`)}
      </span>
      {documentCount > 0 ? (
        <span className="collab-thread-stat" data-compact={compact ? 'true' : undefined}>
              {bilingual(`${documentCount} 份资料`, `${documentCount} docs`)}
        </span>
      ) : null}
      {aiOutputCount > 0 ? (
        <span className="collab-thread-stat" data-compact={compact ? 'true' : undefined}>
              {bilingual(`${aiOutputCount} 个 AI 产出`, `${aiOutputCount} AI outputs`)}
        </span>
      ) : null}
    </div>
  );
}

function ThreadRootCard({ message }: { message: ChannelRenderMessage }) {
  const authorLabel =
    message.author_type === 'agent'
      ? normalizeAgentDisplayName(message.author_name)
      : message.author_name;
  return (
    <div className="rounded-[12px] bg-[hsl(var(--ui-surface-panel-strong))/0.34] px-3 py-2.5">
      <div className="flex items-center gap-2 text-[10px] text-muted-foreground">
            <span className="font-medium uppercase tracking-[0.08em]">{bilingual('协作起点', 'Collaboration start')}</span>
        <span>·</span>
        <span className="truncate">{authorLabel}</span>
        <span className="ml-auto shrink-0">{formatDateTime(message.created_at)}</span>
      </div>
      <div className="mt-1 line-clamp-2 whitespace-pre-wrap break-words text-[11px] leading-5 text-foreground">
        {message.content_text}
      </div>
    </div>
  );
}

function ThreadFlowMessage({
  message,
  isOwn,
  showFullDiagnostics,
}: {
  message: ChannelRenderMessage;
  isOwn: boolean;
  showFullDiagnostics: boolean;
}) {
  const diagnostics = getRuntimeDiagnostics(message);
  if (message.author_type === 'system') {
    return <ChannelSystemBubble message={message} showFullDiagnostics={showFullDiagnostics} />;
  }

  if (message.author_type === 'agent') {
    const codingPayload = getCodingCardPayload(message);
    return (
      <div>
      <ChatMessageBubble
        role="assistant"
        content={message.content_text}
          thinking={message.thinking}
          toolCalls={message.toolCalls}
          turn={message.turn}
          compaction={message.compaction}
          isStreaming={message.isStreaming}
        timestamp={new Date(message.created_at)}
        agentName={normalizeAgentDisplayName(message.author_name)}
          />
          {codingPayload ? (
            <div className="mt-2 flex justify-start">
              <div className="max-w-[86%]">
                <CodingCardDetails payload={codingPayload} />
              </div>
            </div>
          ) : null}
          {diagnostics ? (
            <div className="mt-2 flex justify-start">
            <div className="collab-diagnostic-note">
              <RuntimeDiagnosticDisclosure diagnostics={diagnostics} showFull={showFullDiagnostics} />
            </div>
          </div>
        ) : null}
      </div>
    );
  }

  if (isOwn) {
    return (
      <ChatMessageBubble
        role="user"
        content={message.content_text}
        timestamp={new Date(message.created_at)}
        userName={message.author_name}
      />
    );
  }

  return (
    <div className="mb-3">
      <div className="mb-1 px-1 text-[10px] text-muted-foreground">{message.author_name}</div>
      <div className="max-w-[86%] rounded-[14px] bg-[hsl(var(--ui-surface-panel-strong))/0.26] px-3 py-2">
        <div className="whitespace-pre-wrap break-words text-[12px] leading-5 text-foreground">
          {message.content_text}
        </div>
        <div className="mt-1.5 text-[10px] text-muted-foreground">{formatDateTime(message.created_at)}</div>
      </div>
    </div>
  );
}

export function TeamChannelsPanel({
  teamId,
  initialChannelId = null,
  initialThreadRootId = null,
}: TeamChannelsPanelProps) {
  useTranslation();
  const navigate = useNavigate();
  const { user, isAdmin } = useAuth();
  const { addToast } = useToast();
  const isMobile = useIsMobile();
  const [channels, setChannels] = useState<ChatChannelSummary[]>([]);
  const [channelSearch, setChannelSearch] = useState('');
  const [selectedChannelId, setSelectedChannelId] = useState<string | null>(null);
  const [channelDetail, setChannelDetail] = useState<ChatChannelDetail | null>(null);
  const [messages, setMessages] = useState<ChannelRenderMessage[]>([]);
  const [surfaceView, setSurfaceView] = useState<ChannelDisplayView>('update');
  const [workStatusFilter, setWorkStatusFilter] = useState<CollaborationStatusFilter>('all');
  const [workSurfaceFilter, setWorkSurfaceFilter] = useState<CollaborationSurfaceFilter>('all');
  const [desktopThreadMode, setDesktopThreadMode] = useState(false);
  const [threadRootId, setThreadRootId] = useState<string | null>(null);
  const [threadMessages, setThreadMessages] = useState<ChannelRenderMessage[]>([]);
  const [threadRootMessage, setThreadRootMessage] = useState<ChannelRenderMessage | null>(null);
  const [threadRuntime, setThreadRuntime] = useState<ChatChannelThread['thread_runtime'] | null>(null);
  const [threadDelegationRuntime, setThreadDelegationRuntime] =
    useState<DelegationRuntime | null>(null);
  const [workspaceCodeFiles, setWorkspaceCodeFiles] = useState<string[]>([]);
  const [workspaceCodeFilesRootPath, setWorkspaceCodeFilesRootPath] = useState<string | null>(null);
  const [workspaceCodeFilesTruncated, setWorkspaceCodeFilesTruncated] = useState(false);
  const [workspaceCodeFilesLoading, setWorkspaceCodeFilesLoading] = useState(false);
  const [workspaceCodeFilesError, setWorkspaceCodeFilesError] = useState<string | null>(null);
  const [threadRootExpanded, setThreadRootExpanded] = useState(false);
  const [members, setMembers] = useState<ChatChannelMember[]>([]);
  const [teamMembers, setTeamMembers] = useState<TeamMember[]>([]);
  const [channelDocuments, setChannelDocuments] = useState<DocumentSummary[]>([]);
  const [loadingChannelDocuments, setLoadingChannelDocuments] = useState(false);
  const [channelAiOutputs, setChannelAiOutputs] = useState<DocumentSummary[]>([]);
  const [loadingChannelAiOutputs, setLoadingChannelAiOutputs] = useState(false);
  const [sidePanelMode, setSidePanelMode] = useState<SidePanelMode | null>(null);
  const [promotingDocId, setPromotingDocId] = useState<string | null>(null);
  const [publishingDocId, setPublishingDocId] = useState<string | null>(null);
  const [publishDialogOpen, setPublishDialogOpen] = useState(false);
  const [publishTargetDoc, setPublishTargetDoc] = useState<DocumentSummary | null>(null);
  const [publishName, setPublishName] = useState('');
  const [publishFolderPath, setPublishFolderPath] = useState('/');
  const [folderTree, setFolderTree] = useState<FolderTreeNode[]>([]);
  const [foldersLoading, setFoldersLoading] = useState(false);
  const [visibleAgents, setVisibleAgents] = useState<TeamAgent[]>([]);
  const [loadingChannels, setLoadingChannels] = useState(true);
  const [loadingMessages, setLoadingMessages] = useState(false);
  const [sending, setSending] = useState(false);
  const [composeText, setComposeText] = useState('');
  const [threadComposeText, setThreadComposeText] = useState('');
  const [activeComposerMention, setActiveComposerMention] = useState<ActiveComposerMention | null>(null);
  const [activeComposerPicker, setActiveComposerPicker] = useState<ActiveComposerPicker | null>(null);
  const [composerMenuViewport, setComposerMenuViewport] = useState<ComposerMenuViewport | null>(null);
  const [selectedAgentId, setSelectedAgentId] = useState(DEFAULT_CHANNEL_AGENT_VALUE);
  const [attachedDocs, setAttachedDocs] = useState<DocumentSummary[]>([]);
  const [pendingDocIds, setPendingDocIds] = useState<string[]>([]);
  const [showDocPicker, setShowDocPicker] = useState(false);
  const [showCapabilityPicker, setShowCapabilityPicker] = useState(false);
  const [capabilityCatalog, setCapabilityCatalog] =
    useState<ComposerCapabilitiesCatalog | null>(null);
  const [capabilityLoading, setCapabilityLoading] = useState(false);
  const [capabilityError, setCapabilityError] = useState<string | null>(null);
  const [capabilityDetailKey, setCapabilityDetailKey] = useState<string | null>(null);
  const [selectedCapabilityRefs, setSelectedCapabilityRefs] = useState<string[]>([]);
  const [composerToolsOpen, setComposerToolsOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [onboardingOpen, setOnboardingOpen] = useState(false);
  const [membersOpen, setMembersOpen] = useState(false);
  const [workspaceGovernanceAction, setWorkspaceGovernanceAction] = useState<"restore" | "archive" | "delete" | null>(null);
  const [autonomyGuideOpen, setAutonomyGuideOpen] = useState(false);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [deleteMode, setDeleteMode] = useState<ChatChannelDeleteMode>('preserve_documents');
  const [deletingChannel, setDeletingChannel] = useState(false);
  const [pendingCollaborationAction, setPendingCollaborationAction] =
    useState<PendingCollaborationActionConfirm | null>(null);
  const [confirmingCollaborationAction, setConfirmingCollaborationAction] = useState(false);
  const [form, setForm] = useState<ChannelFormState>(emptyForm());
  const [newMemberId, setNewMemberId] = useState('');
  const [newMemberRole, setNewMemberRole] = useState<'member' | 'manager'>('member');
  const [error, setError] = useState<string | null>(null);
  const [uploadingDocument, setUploadingDocument] = useState(false);
  const currentTeamMemberRole = useMemo(
    () => teamMembers.find((member) => member.userId === user?.id)?.role || null,
    [teamMembers, user?.id],
  );
  const canViewFullDiagnostics =
    isAdmin || currentTeamMemberRole === 'owner' || currentTeamMemberRole === 'admin';
  const streamRef = useRef<EventSource | null>(null);
  const streamThreadRootRef = useRef<string | null>(null);
  const lastEventIdRef = useRef<number | null>(null);
  const selectedChannelIdRef = useRef<string | null>(null);
  const activeThreadRootIdRef = useRef<string | null>(null);
  const messageScrollRef = useRef<HTMLDivElement | null>(null);
  const messageEndRef = useRef<HTMLDivElement | null>(null);
  const threadScrollRef = useRef<HTMLDivElement | null>(null);
  const threadEndRef = useRef<HTMLDivElement | null>(null);
  const stickMainToBottomRef = useRef(true);
  const stickThreadToBottomRef = useRef(true);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const rootComposerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const threadComposerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const rootComposerPickerRef = useRef<HTMLDivElement | null>(null);
  const threadComposerPickerRef = useRef<HTMLDivElement | null>(null);
  const composerMenuRef = useRef<HTMLDivElement | null>(null);
  const rootComposerSelectionRef = useRef<{ start: number; end: number }>({ start: 0, end: 0 });
  const threadComposerSelectionRef = useRef<{ start: number; end: number }>({ start: 0, end: 0 });

  const closeStream = useCallback((options?: { resetCursor?: boolean }) => {
    streamRef.current?.close();
    streamRef.current = null;
    if (options?.resetCursor) {
      lastEventIdRef.current = null;
    }
    streamThreadRootRef.current = null;
  }, []);

  const updateEventCursor = useCallback((event: MessageEvent) => {
    const next = Number.parseInt(event.lastEventId || '', 10);
    if (Number.isFinite(next) && next > 0) {
      lastEventIdRef.current = next;
    }
  }, []);

  const isNearBottom = useCallback((node: HTMLDivElement | null) => {
    if (!node) return true;
    return node.scrollHeight - node.scrollTop - node.clientHeight < 140;
  }, []);

  const scrollToBottom = useCallback((endNode: HTMLDivElement | null) => {
    if (!endNode) return;
    endNode.scrollIntoView({ behavior: 'auto', block: 'end' });
  }, []);

  const autoResizeTextarea = useCallback((node: HTMLTextAreaElement | null, minHeight: number) => {
    if (!node) return;
    const maxHeight = 220;
    node.style.height = '0px';
    const nextHeight = Math.max(minHeight, Math.min(node.scrollHeight, maxHeight));
    node.style.height = `${nextHeight}px`;
    node.style.overflowY = node.scrollHeight > maxHeight ? 'auto' : 'hidden';
  }, []);

  const pairedAssistantByRootId = useMemo(() => {
    const roots = new Set(
      messages
        .filter((message) => message.author_type === 'user' && !message.thread_root_id)
        .map((message) => message.message_id),
    );
    const map = new Map<string, ChannelRenderMessage>();
    messages.forEach((message) => {
      if (
        message.author_type === 'agent' &&
        !message.thread_root_id &&
        message.parent_message_id &&
        roots.has(message.parent_message_id) &&
        !map.has(message.parent_message_id)
      ) {
        map.set(message.parent_message_id, message);
      }
    });
    return map;
  }, [messages]);

  const mainChannelMessages = useMemo(() => {
    const hiddenAssistantIds = new Set(
      Array.from(pairedAssistantByRootId.values()).map((message) => message.message_id),
    );
    return messages.filter((message) => !hiddenAssistantIds.has(message.message_id));
  }, [messages, pairedAssistantByRootId]);

  const collaborationRootMessages = useMemo(
    () =>
      mainChannelMessages.filter(
        (message) =>
          message.display_kind === 'collaboration'
          || (
            message.display_kind === 'suggestion'
            && message.display_status === 'rejected'
            && typeof message.metadata?.linked_collaboration_id !== 'string'
          ),
      ),
    [mainChannelMessages],
  );

  const threadAiOutputCountMap = useMemo(() => {
    const counts = new Map<string, number>();
    channelAiOutputs.forEach((doc) => {
      if (!doc.source_thread_root_id) {
        return;
      }
      counts.set(doc.source_thread_root_id, (counts.get(doc.source_thread_root_id) || 0) + 1);
    });
    return counts;
  }, [channelAiOutputs]);

  const threadDocumentCountMap = useMemo(() => {
    const counts = new Map<string, number>();
    mainChannelMessages.forEach((message) => {
      const rootId = message.thread_root_id || message.message_id;
      const docCount = getAttachedDocumentIds(message).length;
      counts.set(rootId, docCount);
    });
    pairedAssistantByRootId.forEach((message, rootId) => {
      const docCount = getAttachedDocumentIds(message).length;
      counts.set(rootId, Math.max(counts.get(rootId) || 0, docCount));
    });
    return counts;
  }, [mainChannelMessages, pairedAssistantByRootId]);

  const currentThreadDocumentCount = useMemo(() => {
    const ids = new Set<string>();
    getAttachedDocumentIds(threadRootMessage).forEach((id) => ids.add(id));
    threadMessages.forEach((message) => {
      getAttachedDocumentIds(message).forEach((id) => ids.add(id));
    });
    return ids.size;
  }, [threadMessages, threadRootMessage]);

  const currentThreadAiOutputCount = useMemo(() => {
    if (!threadRootId) {
      return 0;
    }
    return threadAiOutputCountMap.get(threadRootId) || 0;
  }, [threadAiOutputCountMap, threadRootId]);

  const currentThreadReplyCount = useMemo(() => {
    return threadRootMessage?.reply_count ?? threadMessages.length;
  }, [threadMessages.length, threadRootMessage?.reply_count]);

  const currentThreadSummary = useMemo(() => {
    const raw = threadRootMessage?.content_text?.trim() || '';
    if (!raw) {
      return bilingual('围绕这件事继续协作', 'Continue collaborating on this work');
    }
    return raw.length > 88 ? `${raw.slice(0, 88)}…` : raw;
  }, [threadRootMessage?.content_text]);

  const currentRootFilter = useMemo(
    () => ({
      display_kind: undefined as ChatChannelDisplayKind | undefined,
      display_status: undefined as ChatChannelDisplayStatus | undefined,
    }),
    [],
  );

  useEffect(() => {
    setWorkSurfaceFilter('all');
    setWorkStatusFilter('all');
  }, [selectedChannelId]);

  const rootComposerIntent = useMemo(() => {
    if (surfaceView === 'work') return 'work';
    if (
      selectedCapabilityRefs.length ||
      pendingDocIds.length ||
      (selectedAgentId && selectedAgentId !== DEFAULT_CHANNEL_AGENT_VALUE)
    ) {
      return 'work';
    }
    return 'update';
  }, [pendingDocIds.length, selectedAgentId, selectedCapabilityRefs.length, surfaceView]);

  const loadChannels = useCallback(async () => {
    setLoadingChannels(true);
    try {
      const [channelList, agents, teamMembersRes] = await Promise.all([
        chatApi.listChannels(teamId),
        fetchVisibleChatAgents(teamId),
        apiClient.getMembers(teamId),
      ]);
      setVisibleAgents(agents);
      setTeamMembers(teamMembersRes.members);
      setChannels(channelList);
      setSelectedChannelId((current) =>
        current && channelList.some((item) => item.channel_id === current)
          ? current
          : channelList[0]?.channel_id || null,
      );
      setError(null);
    } catch (loadError) {
      console.error('Failed to load team channels:', loadError);
      setError(bilingual('当前无法读取团队频道，请稍后再试。', 'Unable to load team channels right now. Please try again later.'));
    } finally {
      setLoadingChannels(false);
    }
  }, [teamId]);

  const loadChannel = useCallback(async (
    channelId: string,
    options?: {
      silent?: boolean;
      preserveSelectedAgent?: boolean;
      display_kind?: ChatChannelDisplayKind;
      display_status?: ChatChannelDisplayStatus;
      surface?: ChatChannelMessageSurface;
      thread_state?: ChatChannelThreadState;
    },
  ) => {
    if (!options?.silent) {
      setLoadingMessages(true);
    }
    try {
      const [detail, rootMessages, memberList] = await Promise.all([
        chatApi.getChannel(channelId),
        chatApi.listChannelMessages(channelId, {
          display_kind: options?.display_kind,
          display_status: options?.display_status,
          surface: options?.surface,
          thread_state: options?.thread_state,
        }),
        chatApi.listChannelMembers(channelId),
      ]);
      setChannelDetail(detail);
      setChannels((prev) =>
        prev.map((item) => (item.channel_id === detail.channel_id ? detail : item)),
      );
      setMembers(memberList);
      setMessages(rootMessages.map(fromChannelMessage));
      setSelectedAgentId((current) => {
        if (
          options?.preserveSelectedAgent &&
          current &&
          current !== DEFAULT_CHANNEL_AGENT_VALUE
        ) {
          return current;
        }
        return DEFAULT_CHANNEL_AGENT_VALUE;
      });
      setError(null);
      await chatApi.markChannelRead(channelId);
      if (detail.document_folder_path) {
        setLoadingChannelDocuments(true);
      } else {
        setChannelDocuments([]);
      }
      setLoadingChannelAiOutputs(true);
      try {
        const [docs, aiOutputs] = await Promise.all([
          detail.document_folder_path
            ? documentApi.listDocuments(teamId, 1, 8, detail.document_folder_path)
            : Promise.resolve({ items: [], total: 0, page: 1, limit: 8, total_pages: 0 }),
          documentApi.listAiWorkbench(teamId, {
            sourceSpaceType: 'team_channel',
            sourceChannelId: detail.channel_id,
            page: 1,
            limit: 8,
          }),
        ]);
        setChannelDocuments(docs.items);
        setChannelAiOutputs(
          aiOutputs.items.filter((doc) => {
            if (detail.document_folder_path && doc.folder_path === detail.document_folder_path) {
              return false;
            }
            if (doc.status === 'accepted' || doc.status === 'archived' || doc.status === 'superseded') {
              return false;
            }
            return true;
          }),
        );
      } catch (docError) {
        console.error('Failed to load channel document views:', docError);
        setChannelDocuments([]);
        setChannelAiOutputs([]);
      } finally {
        setLoadingChannelDocuments(false);
        setLoadingChannelAiOutputs(false);
      }
    } catch (loadError) {
      console.error('Failed to load channel:', loadError);
      setError(bilingual('当前无法读取频道详情，请稍后再试。', 'Unable to load channel details right now. Please try again later.'));
    } finally {
      if (!options?.silent) {
        setLoadingMessages(false);
      }
    }
  }, []);

  const loadThread = useCallback(async (channelId: string, rootId: string) => {
    try {
      const thread = await chatApi.getChannelThread(channelId, rootId);
      const rootMessage = fromChannelMessage(thread.root_message);
      const threadReplies = thread.messages.map(fromChannelMessage);
      const pairedAssistant = pairedAssistantByRootId.get(rootId);
      const mergedReplies = pairedAssistant
        ? [pairedAssistant, ...threadReplies.filter((message) => message.message_id !== pairedAssistant.message_id)]
        : threadReplies;
      setThreadRootMessage(rootMessage);
      setThreadMessages(mergedReplies);
      setThreadRuntime(thread.thread_runtime || null);
      setThreadDelegationRuntime(thread.delegation_runtime || null);
      setThreadRootId(rootId);
    } catch (loadError) {
      console.error('Failed to load channel thread:', loadError);
    }
  }, [pairedAssistantByRootId]);

  useEffect(() => {
    selectedChannelIdRef.current = selectedChannelId;
  }, [selectedChannelId]);

  useEffect(() => {
    activeThreadRootIdRef.current = threadRootId;
  }, [threadRootId]);

  useEffect(() => {
    if (stickMainToBottomRef.current) {
      scrollToBottom(messageEndRef.current);
    }
  }, [messages, scrollToBottom]);

  useEffect(() => {
    if (stickThreadToBottomRef.current) {
      scrollToBottom(threadEndRef.current);
    }
  }, [threadMessages, scrollToBottom]);

  useEffect(() => {
    setAutonomyGuideOpen(false);
  }, [selectedChannelId]);

  useEffect(() => {
    autoResizeTextarea(rootComposerTextareaRef.current, 36);
  }, [autoResizeTextarea, composeText]);

  useEffect(() => {
    autoResizeTextarea(threadComposerTextareaRef.current, 36);
  }, [autoResizeTextarea, threadComposeText]);

  useEffect(() => {
    void loadChannels();
    return () => {
      closeStream({ resetCursor: true });
    };
  }, [closeStream, loadChannels]);

  useEffect(() => {
    if (initialChannelId) {
      setSelectedChannelId(initialChannelId);
    }
  }, [initialChannelId]);

  useEffect(() => {
    closeStream({ resetCursor: true });
    if (!selectedChannelId) {
      setChannelDetail(null);
      setMessages([]);
      setSurfaceView('update');
      setDesktopThreadMode(false);
      setThreadRootId(null);
      setThreadRootMessage(null);
      setThreadMessages([]);
      setThreadRuntime(null);
      setThreadDelegationRuntime(null);
      setThreadRootExpanded(false);
      setSelectedAgentId(DEFAULT_CHANNEL_AGENT_VALUE);
      setAttachedDocs([]);
      setPendingDocIds([]);
      setSelectedCapabilityRefs([]);
      setSidePanelMode(null);
      return;
    }
    setSurfaceView('update');
    setDesktopThreadMode(false);
    setThreadRootId(null);
    setThreadRootMessage(null);
    setThreadMessages([]);
    setThreadRuntime(null);
    setThreadDelegationRuntime(null);
    setThreadRootExpanded(false);
    setSelectedAgentId(DEFAULT_CHANNEL_AGENT_VALUE);
    setAttachedDocs([]);
    setPendingDocIds([]);
    setSelectedCapabilityRefs([]);
    setSidePanelMode(null);
    void loadChannel(selectedChannelId, {
      display_kind: currentRootFilter.display_kind,
      display_status: currentRootFilter.display_status,
    });
  }, [closeStream, loadChannel, selectedChannelId]);

  useEffect(() => {
    if (!selectedChannelId) {
      return;
    }
    void loadChannel(selectedChannelId, {
      silent: true,
      preserveSelectedAgent: true,
      display_kind: currentRootFilter.display_kind,
      display_status: currentRootFilter.display_status,
    });
  }, [currentRootFilter.display_kind, currentRootFilter.display_status, loadChannel, selectedChannelId]);

  useEffect(() => {
    if (!initialThreadRootId || !selectedChannelId || selectedChannelId !== initialChannelId) {
      return;
    }
    setSurfaceView('work');
    if (!isMobile) {
      setDesktopThreadMode(true);
    }
    if (isMobile) {
      setSidePanelMode('thread');
    }
    void loadThread(selectedChannelId, initialThreadRootId);
  }, [initialChannelId, initialThreadRootId, isMobile, loadThread, selectedChannelId]);

  useEffect(() => {
    if (isMobile || !selectedChannelId) {
      return;
    }
    if (!desktopThreadMode) {
      return;
    }
    if (collaborationRootMessages.length === 0) {
      setThreadRootId(null);
      setThreadRootMessage(null);
      setThreadMessages([]);
      setThreadRuntime(null);
      setThreadDelegationRuntime(null);
      return;
    }
    if (
      threadRootId &&
      collaborationRootMessages.some((message) => message.message_id === threadRootId)
    ) {
      return;
    }
    void loadThread(selectedChannelId, collaborationRootMessages[0].message_id);
  }, [collaborationRootMessages, desktopThreadMode, isMobile, loadThread, selectedChannelId, threadRootId]);

  const resolvedComposerAgentId = useMemo(() => {
    if (!channelDetail?.default_agent_id) {
      return '';
    }
    return selectedAgentId && selectedAgentId !== DEFAULT_CHANNEL_AGENT_VALUE
      ? selectedAgentId
      : channelDetail.default_agent_id;
  }, [channelDetail?.default_agent_id, selectedAgentId]);

  const selectableRecipientMembers = useMemo(() => {
    const channelMemberIds = new Set(members.map((member) => member.user_id));
    return teamMembers.filter(
      (member) => member.userId !== user?.id && channelMemberIds.has(member.userId),
    );
  }, [members, teamMembers, user?.id]);

  const mentionOptions = useMemo<ComposerMentionOption[]>(() => {
    const memberOptions = selectableRecipientMembers.map((member) => ({
      mention_type: 'member' as const,
      target_id: member.userId,
      label: member.displayName || member.userId,
      subtitle: bilingual(`成员 · ${member.displayName || member.userId}`, `Member · ${member.displayName || member.userId}`),
    }));
    const agentOptions = visibleAgents.map((agent) => ({
      mention_type: 'agent' as const,
      target_id: agent.id,
      label: normalizeAgentDisplayName(agent.name) || agent.id,
      subtitle: `Agent · ${normalizeAgentDisplayName(agent.name) || agent.id}`,
    }));
    const raw = [...memberOptions, ...agentOptions];
    const labelCounts = new Map<string, number>();
    raw.forEach((item) => {
      labelCounts.set(item.label, (labelCounts.get(item.label) || 0) + 1);
    });
    return raw.map((item) => ({
      key: `${item.mention_type}:${item.target_id}`,
      ...item,
      insertLabel:
        (labelCounts.get(item.label) || 0) > 1
          ? `${item.label}${item.mention_type === 'agent' ? bilingual('（Agent）', ' (Agent)') : bilingual('（成员）', ' (Member)')}`
          : item.label,
    }));
  }, [selectableRecipientMembers, visibleAgents]);

  const rootComposerPickerOptions = useMemo<ComposerPickerOption[]>(() => mentionOptions.map((option) => ({
    key: option.key,
    value: option.key,
    kind: option.mention_type,
    label: `${option.mention_type === 'agent' ? bilingual('AI', 'AI') : bilingual('成员', 'Member')} · ${option.insertLabel}`,
  })), [mentionOptions]);

  const threadComposerPickerOptions = rootComposerPickerOptions;

  const rootComposerMentions = useMemo(
    () => deriveMentionsFromText(composeText, mentionOptions),
    [composeText, mentionOptions],
  );
  const rootComposerAgentMentions = useMemo(
    () => rootComposerMentions.filter((item) => item.mention_type === 'agent'),
    [rootComposerMentions],
  );
  const rootComposerMentionedAgent = useMemo(() => {
    if (rootComposerAgentMentions.length !== 1) {
      return null;
    }
    return (
      visibleAgents.find((agent) => agent.id === rootComposerAgentMentions[0].target_id) || null
    );
  }, [rootComposerAgentMentions, visibleAgents]);
  const rootStartsTemporaryCollaboration = rootComposerAgentMentions.length === 1;
  const rootMentionedAgentLabel = useMemo(
    () =>
      rootComposerMentionedAgent
        ? normalizeAgentDisplayName(rootComposerMentionedAgent.name) || rootComposerMentionedAgent.id
        : '',
    [rootComposerMentionedAgent],
  );

  const rootComposerTargetLabel = useMemo(() => {
    const latest = findLatestMentionOptionInText(composeText, mentionOptions);
    if (!latest) {
      return bilingual('@成员 / Agent', '@Member / Agent');
    }
    return `${latest.mention_type === 'agent' ? bilingual('AI', 'AI') : bilingual('成员', 'Member')} · ${latest.insertLabel}`;
  }, [composeText, mentionOptions]);

  const threadComposerTargetLabel = useMemo(() => {
    const latest = findLatestMentionOptionInText(threadComposeText, mentionOptions);
    if (!latest) {
      return bilingual('@成员 / Agent', '@Member / Agent');
    }
    return `${latest.mention_type === 'agent' ? bilingual('AI', 'AI') : bilingual('成员', 'Member')} · ${latest.insertLabel}`;
  }, [mentionOptions, threadComposeText]);

  const updateComposerMention = useCallback((
    composer: ComposerMentionComposer,
    text: string,
    caret: number,
  ) => {
    const next = detectMentionQuery(text, caret);
    if (!next) {
      setActiveComposerMention((prev) => (prev?.composer === composer ? null : prev));
      return;
    }
    setActiveComposerPicker((prev) => (prev?.composer === composer ? null : prev));
    const normalizedQuery = next.query.trim().toLowerCase();
    const filtered = mentionOptions.filter((option) => {
      if (!normalizedQuery) {
        return true;
      }
      return option.insertLabel.toLowerCase().includes(normalizedQuery)
        || option.label.toLowerCase().includes(normalizedQuery)
        || option.subtitle.toLowerCase().includes(normalizedQuery);
    }).slice(0, 7);
    if (filtered.length === 0) {
      setActiveComposerMention((prev) => (prev?.composer === composer ? null : prev));
      return;
    }
    setActiveComposerMention((prev) => ({
      composer,
      query: next.query,
      start: next.start,
      end: next.end,
      options: filtered,
      selectedIndex:
        prev?.composer === composer
          ? Math.min(prev.selectedIndex, filtered.length - 1)
          : 0,
    }));
  }, [mentionOptions]);

  const closeComposerMention = useCallback((composer?: ComposerMentionComposer) => {
    setActiveComposerMention((prev) => {
      if (!prev) return prev;
      if (composer && prev.composer !== composer) {
        return prev;
      }
      return null;
    });
  }, []);

  const setComposerSelection = useCallback((
    composer: ComposerMentionComposer,
    start: number,
    end = start,
  ) => {
    const safeSelection = { start: Math.max(0, start), end: Math.max(0, end) };
    if (composer === 'root') {
      rootComposerSelectionRef.current = safeSelection;
    } else {
      threadComposerSelectionRef.current = safeSelection;
    }
  }, []);

  const insertComposerMention = useCallback((composer: ComposerMentionComposer, option: ComposerMentionOption) => {
    const sourceText = composer === 'root' ? composeText : threadComposeText;
    const mentionState = activeComposerMention?.composer === composer ? activeComposerMention : null;
    const selection = composer === 'root'
      ? rootComposerSelectionRef.current
      : threadComposerSelectionRef.current;
    const replaceStart = mentionState?.start ?? selection.start ?? sourceText.length;
    const replaceEnd = mentionState?.end ?? selection.end ?? replaceStart;
    const before = sourceText.slice(0, replaceStart);
    const after = sourceText.slice(replaceEnd);
    const needsLeadingSpace = before.length > 0 && !/[\s\u3000]/.test(before[before.length - 1]);
    const insertion = `${needsLeadingSpace ? ' ' : ''}@${option.insertLabel} `;
    const nextText = `${before}${insertion}${after}`;
    const nextCaret = before.length + insertion.length;
    if (composer === 'root') {
      setComposeText(nextText);
    } else {
      setThreadComposeText(nextText);
    }
    setComposerSelection(composer, nextCaret, nextCaret);
    setActiveComposerMention((prev) => (prev?.composer === composer ? null : prev));
    const ref = composer === 'root' ? rootComposerTextareaRef.current : threadComposerTextareaRef.current;
    window.setTimeout(() => {
      ref?.focus();
      ref?.setSelectionRange(nextCaret, nextCaret);
    }, 0);
  }, [activeComposerMention, composeText, setComposerSelection, threadComposeText]);

  const toggleComposerPicker = useCallback((composer: ComposerMentionComposer) => {
    const options = composer === 'root' ? rootComposerPickerOptions : threadComposerPickerOptions;
    setActiveComposerMention((prev) => (prev?.composer === composer ? null : prev));
    setActiveComposerPicker((prev) => {
      if (prev?.composer === composer) {
        return null;
      }
      return {
        composer,
        options,
        selectedIndex: 0,
      };
    });
  }, [
    rootComposerPickerOptions,
    threadComposerPickerOptions,
  ]);

  const applyComposerPickerSelection = useCallback((
    composer: ComposerMentionComposer,
    option: ComposerPickerOption,
  ) => {
    const mentionOption = mentionOptions.find((item) => item.key === option.value);
    if (!mentionOption) {
      setActiveComposerPicker((prev) => (prev?.composer === composer ? null : prev));
      return;
    }
    insertComposerMention(composer, mentionOption);
    setActiveComposerPicker((prev) => (prev?.composer === composer ? null : prev));
  }, [insertComposerMention, mentionOptions]);

  const handleComposerMentionKeyDown = useCallback((
    composer: ComposerMentionComposer,
    event: KeyboardEvent<HTMLElement>,
  ) => {
    const mentionState = activeComposerMention?.composer === composer ? activeComposerMention : null;
    const pickerState = activeComposerPicker?.composer === composer ? activeComposerPicker : null;
    const state = mentionState || pickerState;
    if (!state) {
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      if (mentionState) {
        setActiveComposerMention((prev) => prev && prev.composer === composer
          ? { ...prev, selectedIndex: (prev.selectedIndex + 1) % prev.options.length }
          : prev);
      } else {
        setActiveComposerPicker((prev) => prev && prev.composer === composer
          ? { ...prev, selectedIndex: (prev.selectedIndex + 1) % prev.options.length }
          : prev);
      }
      return;
    }
    if (event.key === 'ArrowUp') {
      event.preventDefault();
      if (mentionState) {
        setActiveComposerMention((prev) => prev && prev.composer === composer
          ? { ...prev, selectedIndex: (prev.selectedIndex - 1 + prev.options.length) % prev.options.length }
          : prev);
      } else {
        setActiveComposerPicker((prev) => prev && prev.composer === composer
          ? { ...prev, selectedIndex: (prev.selectedIndex - 1 + prev.options.length) % prev.options.length }
          : prev);
      }
      return;
    }
    if (event.key === 'Enter' || event.key === 'Tab') {
      event.preventDefault();
      if (mentionState) {
        const option = mentionState.options[mentionState.selectedIndex];
        if (option) {
          insertComposerMention(composer, option);
        }
      } else {
        const option = pickerState?.options[pickerState.selectedIndex];
        if (option) {
          applyComposerPickerSelection(composer, option);
        }
      }
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      closeComposerMention(composer);
      setActiveComposerPicker((prev) => (prev?.composer === composer ? null : prev));
    }
  }, [activeComposerMention, activeComposerPicker, applyComposerPickerSelection, closeComposerMention, insertComposerMention]);

  const handleRootComposerChange = useCallback((event: ChangeEvent<HTMLTextAreaElement>) => {
    const next = event.target.value;
    setComposeText(next);
    setComposerSelection('root', event.target.selectionStart ?? next.length, event.target.selectionEnd ?? event.target.selectionStart ?? next.length);
    setActiveComposerPicker((prev) => (prev?.composer === 'root' ? null : prev));
    updateComposerMention('root', next, event.target.selectionStart ?? next.length);
  }, [setComposerSelection, updateComposerMention]);

  const handleThreadComposerChange = useCallback((event: ChangeEvent<HTMLTextAreaElement>) => {
    const next = event.target.value;
    setThreadComposeText(next);
    setComposerSelection('thread', event.target.selectionStart ?? next.length, event.target.selectionEnd ?? event.target.selectionStart ?? next.length);
    setActiveComposerPicker((prev) => (prev?.composer === 'thread' ? null : prev));
    updateComposerMention('thread', next, event.target.selectionStart ?? next.length);
  }, [setComposerSelection, updateComposerMention]);

  const handleRootComposerSelect = useCallback((event: SyntheticEvent<HTMLTextAreaElement>) => {
    const target = event.currentTarget;
    setComposerSelection('root', target.selectionStart ?? target.value.length, target.selectionEnd ?? target.selectionStart ?? target.value.length);
    updateComposerMention('root', target.value, target.selectionStart ?? target.value.length);
  }, [setComposerSelection, updateComposerMention]);

  const handleThreadComposerSelect = useCallback((event: SyntheticEvent<HTMLTextAreaElement>) => {
    const target = event.currentTarget;
    setComposerSelection('thread', target.selectionStart ?? target.value.length, target.selectionEnd ?? target.selectionStart ?? target.value.length);
    updateComposerMention('thread', target.value, target.selectionStart ?? target.value.length);
  }, [setComposerSelection, updateComposerMention]);

  const updateComposerMenuViewport = useCallback(() => {
    const activeComposer = activeComposerMention?.composer || activeComposerPicker?.composer;
    if (!activeComposer) {
      setComposerMenuViewport(null);
      return;
    }
    const anchor = activeComposer === 'root' ? rootComposerPickerRef.current : threadComposerPickerRef.current;
    if (!anchor) {
      setComposerMenuViewport(null);
      return;
    }
    const rect = anchor.getBoundingClientRect();
    const width = Math.min(Math.max(rect.width, 220), 280);
    const maxLeft = Math.max(12, window.innerWidth - width - 12);
    const left = Math.min(Math.max(rect.left, 12), maxLeft);
    const availableAbove = rect.top - 12;
    const availableBelow = window.innerHeight - rect.bottom - 12;
    const placement: 'top' | 'bottom' =
      availableAbove >= 220 || availableAbove >= availableBelow
        ? 'top'
        : 'bottom';
    setComposerMenuViewport({
      top: placement === 'top' ? rect.top - 8 : rect.bottom + 8,
      left,
      width,
      placement,
    });
  }, [activeComposerMention?.composer, activeComposerPicker?.composer]);

  const renderComposerTargetMenu = useCallback((composer: ComposerMentionComposer) => {
    const mentionState = activeComposerMention?.composer === composer ? activeComposerMention : null;
    const pickerState = activeComposerPicker?.composer === composer ? activeComposerPicker : null;
    if ((!mentionState && !pickerState) || !composerMenuViewport) {
      return null;
    }
    const selectedIndex = mentionState?.selectedIndex ?? pickerState?.selectedIndex ?? 0;
    const items = mentionState
      ? mentionState.options.map((option) => ({
          key: option.key,
          label: `${option.mention_type === 'agent' ? bilingual('AI', 'AI') : bilingual('成员', 'Member')} · ${option.insertLabel}`,
          onSelect: () => insertComposerMention(composer, option),
        }))
      : (pickerState?.options || []).map((option) => ({
          key: option.key,
          label: option.label,
          onSelect: () => applyComposerPickerSelection(composer, option),
        }));
    const style: CSSProperties = {
      position: 'fixed',
      left: composerMenuViewport.left,
      top: composerMenuViewport.top,
      width: composerMenuViewport.width,
      transform: composerMenuViewport.placement === 'top' ? 'translateY(-100%)' : undefined,
    };
    return createPortal(
      <div
        ref={composerMenuRef}
        style={style}
        className="z-[140] inline-flex overflow-hidden rounded-[16px] border border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--popover))/0.98] p-1 shadow-[0_22px_48px_hsl(var(--ui-shadow)/0.18)] backdrop-blur-xl"
      >
        <div className="max-h-[220px] w-full overflow-y-auto">
          {items.map((item, index) => {
            const selected = index === selectedIndex;
            return (
              <button
                key={item.key}
                type="button"
                onMouseDown={(event) => event.preventDefault()}
                onClick={item.onSelect}
                className={`relative flex w-full items-center rounded-[10px] px-3 py-2 pl-8 pr-3 text-left text-[13px] font-medium transition-colors ${
                  selected
                    ? 'bg-[hsl(var(--accent))/0.54] text-[hsl(var(--accent-foreground))]'
                    : 'text-[hsl(var(--foreground))] hover:bg-[hsl(var(--accent))/0.28]'
                }`}
              >
                <span className="absolute left-2 flex h-3.5 w-3.5 items-center justify-center text-[hsl(var(--foreground))]">
                  {selected ? <Check className="h-4 w-4" /> : null}
                </span>
                <span className="truncate">{item.label}</span>
              </button>
            );
          })}
        </div>
      </div>,
      document.body,
    );
  }, [activeComposerMention, activeComposerPicker, applyComposerPickerSelection, composerMenuViewport, insertComposerMention]);

  useEffect(() => {
    if (!activeComposerMention && !activeComposerPicker) {
      setComposerMenuViewport(null);
      return;
    }
    updateComposerMenuViewport();
    const handleViewportUpdate = () => updateComposerMenuViewport();
    window.addEventListener('resize', handleViewportUpdate);
    window.addEventListener('scroll', handleViewportUpdate, true);
    return () => {
      window.removeEventListener('resize', handleViewportUpdate);
      window.removeEventListener('scroll', handleViewportUpdate, true);
    };
  }, [activeComposerMention, activeComposerPicker, updateComposerMenuViewport]);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) {
        return;
      }
      const isInsideRoot =
        rootComposerPickerRef.current?.contains(target)
        || rootComposerTextareaRef.current?.contains(target)
        || composerMenuRef.current?.contains(target);
      const isInsideThread =
        threadComposerPickerRef.current?.contains(target)
        || threadComposerTextareaRef.current?.contains(target)
        || composerMenuRef.current?.contains(target);
      if (!isInsideRoot) {
        closeComposerMention('root');
        setActiveComposerPicker((prev) => (prev?.composer === 'root' ? null : prev));
      }
      if (!isInsideThread) {
        closeComposerMention('thread');
        setActiveComposerPicker((prev) => (prev?.composer === 'thread' ? null : prev));
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
    };
  }, [closeComposerMention]);

  const renderComposerTargetTrigger = useCallback((
    composer: ComposerMentionComposer,
    triggerClassName: string,
  ) => {
    const isRoot = composer === 'root';
    const isOpen =
      activeComposerMention?.composer === composer
      || activeComposerPicker?.composer === composer;
    const label = isRoot ? rootComposerTargetLabel : threadComposerTargetLabel;
    const ref = isRoot ? rootComposerPickerRef : threadComposerPickerRef;
    return (
      <div ref={ref} className="relative">
        <button
          type="button"
          onClick={() => toggleComposerPicker(composer)}
          onKeyDown={(event) => handleComposerMentionKeyDown(composer, event)}
          className={`${triggerClassName} inline-flex w-full items-center justify-between gap-2`}
        >
          <span className="truncate">{label}</span>
          <ChevronRight
            className={`h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform ${
              isOpen ? 'rotate-90' : ''
            }`}
          />
        </button>
        {renderComposerTargetMenu(composer)}
      </div>
    );
  }, [
    activeComposerMention?.composer,
    activeComposerPicker?.composer,
    renderComposerTargetMenu,
    rootComposerTargetLabel,
    threadComposerTargetLabel,
    toggleComposerPicker,
  ]);

  useEffect(() => {
    if (!showCapabilityPicker || !resolvedComposerAgentId) {
      return;
    }
    let active = true;
    setCapabilityLoading(true);
    setCapabilityError(null);
    chatApi
      .getAgentComposerCapabilities(resolvedComposerAgentId)
      .then((catalog) => {
        if (!active) return;
        setCapabilityCatalog(catalog);
      })
      .catch((capabilityLoadError) => {
        console.error('Failed to load channel composer capabilities:', capabilityLoadError);
        if (!active) return;
        setCapabilityCatalog(null);
        setCapabilityError(bilingual('当前无法读取可用技能和扩展，请稍后再试。', 'Unable to load available skills and extensions right now. Please try again later.'));
      })
      .finally(() => {
        if (active) {
          setCapabilityLoading(false);
        }
      });
    return () => {
      active = false;
    };
  }, [resolvedComposerAgentId, showCapabilityPicker]);

  const capabilityRefMap = useMemo(() => {
    const entries = new Map<
      string,
      {
        key: string;
        kind: 'skill' | 'extension';
        name: string;
        displayLineZh: string;
        plainLineZh: string;
        description?: string | null;
        summaryText?: string | null;
        detailText?: string | null;
        detailLang?: string | null;
        detailSource?: string | null;
        badge?: string | null;
      }
    >();

    capabilityCatalog?.skills.forEach((skill) => {
      entries.set(skill.skill_ref, {
        key: `skill:${skill.id}`,
        kind: 'skill',
        name: skill.name,
        displayLineZh: skill.display_line_zh,
        plainLineZh: skill.plain_line_zh,
        description: skill.description,
        summaryText: skill.summary_text,
        detailText: skill.detail_text,
        detailLang: skill.detail_lang,
        detailSource: skill.detail_source,
        badge: skill.version ? `v${skill.version}` : null,
      });
    });

    capabilityCatalog?.extensions.forEach((extension) => {
      entries.set(extension.ext_ref, {
        key: `ext:${extension.runtime_name}`,
        kind: 'extension',
        name: extension.display_name,
        displayLineZh: extension.display_line_zh,
        plainLineZh: extension.plain_line_zh,
        description: extension.description,
        summaryText: extension.summary_text,
        detailText: extension.detail_text,
        detailLang: extension.detail_lang,
        detailSource: extension.detail_source,
        badge: extension.type || extension.class,
      });
    });

    return entries;
  }, [capabilityCatalog]);

  const selectedCapabilityKeys = useMemo(
    () =>
      selectedCapabilityRefs
        .map((ref) => capabilityRefMap.get(ref)?.key)
        .filter((value): value is string => Boolean(value)),
    [capabilityRefMap, selectedCapabilityRefs],
  );

  const selectedCapabilities = useMemo(
    () =>
      selectedCapabilityRefs.map((ref) => {
        const meta = capabilityRefMap.get(ref);
        return {
          ref,
          key: meta?.key ?? ref,
          kind: meta?.kind ?? (ref.startsWith('[[skill:') ? 'skill' : 'extension'),
          name: meta?.name ?? inferCapabilityNameFromRef(ref),
          displayLineZh: meta?.displayLineZh ?? ref,
          plainLineZh: meta?.plainLineZh ?? inferCapabilityNameFromRef(ref),
          description: meta?.description,
          summaryText: meta?.summaryText,
          detailText: meta?.detailText,
          detailLang: meta?.detailLang,
          detailSource: meta?.detailSource,
          badge: meta?.badge,
        };
      }),
    [capabilityRefMap, selectedCapabilityRefs],
  );

  const updateStreamingMessage = useCallback(
    (
      updater: (message: ChannelRenderMessage) => ChannelRenderMessage,
      threadTarget?: string | null,
    ) => {
      if (threadTarget) {
        setThreadMessages((prev) => {
          const next = [...prev];
          for (let index = next.length - 1; index >= 0; index -= 1) {
            if (next[index].author_type === 'agent' && next[index].isStreaming) {
              next[index] = updater(next[index]);
              break;
            }
          }
          return next;
        });
        return;
      }
      setMessages((prev) => {
        const next = [...prev];
        for (let index = next.length - 1; index >= 0; index -= 1) {
          if (next[index].author_type === 'agent' && next[index].isStreaming) {
            next[index] = updater(next[index]);
            break;
          }
        }
        return next;
      });
    },
    [],
  );

  const openStream = useCallback(
    (
      channelId: string,
      threadTarget?: string | null,
      surfaceOverride?: ChatChannelMessageSurface,
      threadStateOverride?: ChatChannelThreadState,
    ) => {
      closeStream();
      streamThreadRootRef.current = threadTarget || null;
      const es = chatApi.streamChannel(channelId, lastEventIdRef.current);
      streamRef.current = es;

      const safeParse = (raw: string) => {
        try {
          return JSON.parse(raw);
        } catch {
          return null;
        }
      };

      const targetThreadRoot = threadTarget || null;
      const reloadDisplayKind: ChatChannelDisplayKind | undefined =
        surfaceOverride === 'activity'
          ? 'discussion'
          : surfaceOverride
            ? 'collaboration'
            : currentRootFilter.display_kind;
      const reloadDisplayStatus =
        surfaceOverride === 'temporary'
          ? 'active'
          : surfaceOverride === 'issue'
            ? 'active'
            : threadStateOverride === 'archived'
              ? 'rejected'
              : currentRootFilter.display_status;
      let streamSettled = false;

      es.addEventListener('text', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data);
        const content = typeof data?.content === 'string' ? data.content : '';
        updateStreamingMessage(
          (message) => ({
            ...message,
            isStreaming: true,
            content_text: `${message.content_text || ''}${content}`,
          }),
          targetThreadRoot,
        );
      });

      es.addEventListener('thinking', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data);
        const content = typeof data?.content === 'string' ? data.content : '';
        updateStreamingMessage(
          (message) => ({
            ...message,
            thinking: `${message.thinking || ''}${content}`,
          }),
          targetThreadRoot,
        );
      });

      es.addEventListener('toolcall', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data);
        if (!data) return;
        updateStreamingMessage(
          (message) => ({
            ...message,
            toolCalls: [
              ...(message.toolCalls || []),
              { id: data.id, name: data.name, status: 'running' },
            ],
          }),
          targetThreadRoot,
        );
      });

      es.addEventListener('toolresult', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data);
        if (!data) return;
        updateStreamingMessage(
          (message) => ({
            ...message,
            toolCalls: (message.toolCalls || []).map((tool) =>
              tool.id === data.id
                ? {
                    ...tool,
                    result: data.content,
                    success: data.success,
                    status: data.success === false ? 'failed' : 'completed',
                  }
                : tool,
            ),
          }),
          targetThreadRoot,
        );
      });

      es.addEventListener('delegation', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data) as DelegationRuntimeEventPayload | null;
        if (!data) return;
        const selectedThread = targetThreadRoot || activeThreadRootIdRef.current;
        if (selectedThread && data.thread_root_id && data.thread_root_id !== selectedThread) {
          return;
        }
        if (!selectedThread && data.thread_root_id) {
          return;
        }
        setThreadDelegationRuntime((prev) => applyDelegationRuntimePatch(prev, data));
      });

      es.addEventListener('turn', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data);
        if (!data) return;
        updateStreamingMessage(
          (message) => ({
            ...message,
            turn: { current: data.current, max: data.max },
          }),
          targetThreadRoot,
        );
      });

      es.addEventListener('compaction', (event) => {
        updateEventCursor(event as MessageEvent);
        const data = safeParse((event as MessageEvent).data);
        if (!data) return;
        updateStreamingMessage(
          (message) => ({
            ...message,
            compaction: {
              strategy: data.strategy,
              before: data.before_tokens,
              after: data.after_tokens,
              phase: data.phase,
              reason: data.reason,
            },
          }),
          targetThreadRoot,
        );
      });

      es.addEventListener('done', async (event) => {
        updateEventCursor(event as MessageEvent);
        streamSettled = true;
        closeStream();
        setSending(false);
        await loadChannel(channelId, {
          silent: true,
          preserveSelectedAgent: true,
          display_kind: reloadDisplayKind,
          display_status: reloadDisplayStatus,
        });
        if (targetThreadRoot) {
          await loadThread(channelId, targetThreadRoot);
        }
      });

      es.onerror = async () => {
        if (streamSettled) {
          return;
        }
        streamSettled = true;
        closeStream();
        setSending(false);
        if (selectedChannelIdRef.current !== channelId) {
          return;
        }
        try {
          await loadChannel(channelId, {
            silent: true,
            preserveSelectedAgent: true,
            display_kind: reloadDisplayKind,
            display_status: reloadDisplayStatus,
          });
          if (targetThreadRoot && activeThreadRootIdRef.current === targetThreadRoot) {
            await loadThread(channelId, targetThreadRoot);
          }
        } catch (streamError) {
          console.error('Failed to reconcile channel after stream interruption:', streamError);
        }
      };
    },
    [closeStream, currentRootFilter.display_kind, currentRootFilter.display_status, loadChannel, loadThread, updateEventCursor, updateStreamingMessage],
  );

  const resetCreateForm = useCallback(() => {
    setForm(emptyForm(visibleAgents[0]?.id || ''));
  }, [visibleAgents]);

  const handleCreateChannel = useCallback(async () => {
    if (!form.name.trim() || !form.defaultAgentId) {
      return;
    }
    try {
        const created = await chatApi.createChannel(teamId, {
          name: form.name,
          description: form.description || null,
          visibility: form.visibility,
          channel_type: form.channelType,
          default_agent_id: form.defaultAgentId,
          member_user_ids: form.visibility === 'team_private' ? form.memberUserIds : [],
          workspace_display_name:
            form.channelType === 'coding' ? (form.workspaceDisplayName || form.name) : null,
          repo_default_branch:
            form.channelType === 'coding' ? (form.repoDefaultBranch || 'main') : null,
        });
      setCreateOpen(false);
      resetCreateForm();
      await loadChannels();
      setSelectedChannelId(created.channel_id);
    } catch (createError) {
      console.error('Failed to create channel:', createError);
      setError(bilingual('创建频道失败，请稍后再试。', 'Failed to create the channel. Please try again later.'));
    }
  }, [form, loadChannels, resetCreateForm, teamId]);

  const handleSaveChannel = useCallback(async () => {
    if (!channelDetail) return;
    try {
        const updated = await chatApi.updateChannel(channelDetail.channel_id, {
          name: form.name,
          description: form.description || null,
          visibility: form.visibility,
          channel_type: form.channelType,
          default_agent_id: form.defaultAgentId,
          workspace_display_name:
            form.channelType === 'coding'
              ? (form.workspaceDisplayName || channelDetail.workspace_display_name || channelDetail.name)
              : null,
          repo_default_branch:
            form.channelType === 'coding'
              ? (form.repoDefaultBranch || channelDetail.repo_default_branch || 'main')
              : null,
          agent_autonomy_mode: form.agentAutonomyMode,
          channel_goal: form.channelGoal || null,
        participant_notes: form.participantNotes || null,
        expected_outputs: form.expectedOutputs || null,
        collaboration_style: form.collaborationStyle || null,
      });
      setSettingsOpen(false);
      setChannelDetail(updated);
      setChannels((prev) =>
        prev.map((item) => (item.channel_id === updated.channel_id ? updated : item)),
      );
    } catch (saveError) {
      console.error('Failed to update channel:', saveError);
      setError(bilingual('保存频道设置失败，请稍后再试。', 'Failed to save channel settings. Please try again later.'));
    }
  }, [channelDetail, form]);

  const handleRestoreDetachedWorkspace = useCallback(async () => {
    const workspaceId = channelDetail?.workspace_governance?.detached_workspace_id;
    if (!workspaceId || workspaceGovernanceAction) return;
    setWorkspaceGovernanceAction('restore');
    try {
      const restored = await chatApi.restoreChannelWorkspace(workspaceId);
      setChannelDetail(restored);
      setChannels((prev) =>
        prev.map((item) => (item.channel_id === restored.channel_id ? restored : item)),
      );
      setSelectedChannelId(restored.channel_id);
      addToast('success', '已恢复原项目工作区并切回编程项目频道');
    } catch (restoreError) {
      console.error('Failed to restore detached workspace:', restoreError);
      setError(bilingual('恢复项目工作区失败，请稍后再试。', 'Failed to restore the project workspace. Please try again later.'));
    } finally {
      setWorkspaceGovernanceAction(null);
    }
  }, [addToast, channelDetail, workspaceGovernanceAction]);

  const handleArchiveDetachedWorkspace = useCallback(async () => {
    const workspaceId = channelDetail?.workspace_governance?.detached_workspace_id;
    if (!workspaceId || workspaceGovernanceAction) return;
    setWorkspaceGovernanceAction('archive');
    try {
      await chatApi.archiveChannelWorkspace(workspaceId);
      await loadChannel(channelDetail.channel_id, { silent: true, preserveSelectedAgent: true });
      addToast('success', '已归档解绑的项目工作区');
    } catch (archiveError) {
      console.error('Failed to archive detached workspace:', archiveError);
      setError(bilingual('归档项目工作区失败，请稍后再试。', 'Failed to archive the project workspace. Please try again later.'));
    } finally {
      setWorkspaceGovernanceAction(null);
    }
  }, [addToast, channelDetail, loadChannel, workspaceGovernanceAction]);

  const handleDeleteDetachedWorkspace = useCallback(async () => {
    const workspaceId = channelDetail?.workspace_governance?.detached_workspace_id;
    if (!workspaceId || workspaceGovernanceAction) return;
    if (!window.confirm(bilingual('这会删除解绑后保留的底层项目工作区、repo 和 worktree。确认继续吗？', 'This will delete the detached workspace, repo, and worktree that were kept after unbinding. Continue?'))) {
      return;
    }
    setWorkspaceGovernanceAction('delete');
    try {
      await chatApi.deleteChannelWorkspace(workspaceId);
      await loadChannel(channelDetail.channel_id, { silent: true, preserveSelectedAgent: true });
      addToast('success', '已删除解绑的项目工作区');
    } catch (deleteError) {
      console.error('Failed to delete detached workspace:', deleteError);
      setError(bilingual('删除项目工作区失败，请确认当前没有活跃线程或会话占用后再试。', 'Failed to delete the project workspace. Make sure no active thread or session is using it, then try again.'));
    } finally {
      setWorkspaceGovernanceAction(null);
    }
  }, [addToast, channelDetail, loadChannel, workspaceGovernanceAction]);

  const handleDeleteChannel = useCallback(async () => {
    if (!channelDetail || deletingChannel) return;
    setDeletingChannel(true);
    try {
      await chatApi.deleteChannel(channelDetail.channel_id, deleteMode);
      setDeleteDialogOpen(false);
      setSettingsOpen(false);
      setThreadRootId(null);
      setThreadRootMessage(null);
      setThreadMessages([]);
      setThreadRuntime(null);
      setThreadDelegationRuntime(null);
      setChannelDetail(null);
      setMessages([]);
      setMembers([]);
      setChannelDocuments([]);
      setChannelAiOutputs([]);
      setSelectedChannelId(null);
      await loadChannels();
      addToast(
        'success',
        deleteMode === 'full_delete' ? bilingual('频道已彻底删除', 'Channel permanently deleted') : bilingual('频道已删除，文档已保留', 'Channel deleted, documents preserved'),
      );
    } catch (deleteError) {
      console.error('Failed to delete channel:', deleteError);
      setError(deleteMode === 'full_delete' ? bilingual('彻底删除频道失败，请稍后再试。', 'Failed to permanently delete the channel. Please try again later.') : bilingual('删除频道失败，请稍后再试。', 'Failed to delete the channel. Please try again later.'));
    } finally {
      setDeletingChannel(false);
    }
  }, [addToast, channelDetail, deleteMode, deletingChannel, loadChannels]);

  const handleAddMember = useCallback(async () => {
    if (!channelDetail || !newMemberId) return;
    try {
      const nextMembers = await chatApi.addChannelMember(channelDetail.channel_id, {
        user_id: newMemberId,
        role: newMemberRole,
      });
      setMembers(nextMembers);
      setNewMemberId('');
    } catch (memberError) {
      console.error('Failed to add channel member:', memberError);
      setError(bilingual('添加成员失败，请稍后再试。', 'Failed to add the member. Please try again later.'));
    }
  }, [channelDetail, newMemberId, newMemberRole]);

  const handleUpdateMemberRole = useCallback(
    async (userId: string, role: 'owner' | 'manager' | 'member') => {
      if (!channelDetail) return;
      try {
        const nextMembers = await chatApi.updateChannelMember(channelDetail.channel_id, userId, {
          role,
        });
        setMembers(nextMembers);
      } catch (memberError) {
        console.error('Failed to update member role:', memberError);
      setError(bilingual('更新成员角色失败，请稍后再试。', 'Failed to update the member role. Please try again later.'));
      }
    },
    [channelDetail],
  );

  const handleRemoveMember = useCallback(
    async (userId: string) => {
      if (!channelDetail) return;
      try {
        const nextMembers = await chatApi.removeChannelMember(channelDetail.channel_id, userId);
        setMembers(nextMembers);
      } catch (memberError) {
        console.error('Failed to remove member:', memberError);
      setError(bilingual('移除成员失败，请稍后再试。', 'Failed to remove the member. Please try again later.'));
      }
    },
    [channelDetail],
  );

  const handleOpenThread = useCallback(
    (message: ChannelRenderMessage) => {
      setSurfaceView('work');
      setWorkSurfaceFilter(message.surface === 'issue' ? 'issue' : 'temporary');
      setDesktopThreadMode(true);
      if (isMobile) {
        setSidePanelMode('thread');
      }
      void loadThread(message.channel_id, message.message_id);
    },
    [isMobile, loadThread],
  );

  const closeThreadPanel = useCallback(() => {
    setDesktopThreadMode(false);
    setThreadRootId(null);
    setThreadRootMessage(null);
    setThreadMessages([]);
    setThreadRuntime(null);
    setThreadDelegationRuntime(null);
    setThreadRootExpanded(false);
    if (isMobile) {
      setSidePanelMode(null);
    }
  }, [isMobile]);

  const handleSelectSurfaceView = useCallback((nextView: ChannelDisplayView) => {
    if (nextView === 'work') {
      setSurfaceView('work');
      setWorkSurfaceFilter('all');
      setWorkStatusFilter('all');
      closeThreadPanel();
      return;
    }
    setSurfaceView('update');
  }, [closeThreadPanel]);

  const requestCollaborationActionConfirm = useCallback(
    (config: PendingCollaborationActionConfirm) => {
      setPendingCollaborationAction(config);
    },
    [],
  );

  const handleConfirmCollaborationAction = useCallback(async () => {
    if (!pendingCollaborationAction) return;
    setConfirmingCollaborationAction(true);
    try {
      await pendingCollaborationAction.onConfirm();
      setPendingCollaborationAction(null);
    } finally {
      setConfirmingCollaborationAction(false);
    }
  }, [pendingCollaborationAction]);

  const handleMarkThreadHighlighted = useCallback(async (message: ChannelRenderMessage) => {
    if (!channelDetail) return;
    if (message.surface === 'issue') {
      addToast('success', '当前协作项已经是正式协作');
      return;
    }
    if (message.surface !== 'temporary') {
      setError(bilingual('只有临时协作可以升级为正式协作。', 'Only temporary collaboration items can be promoted to formal collaboration.'));
      return;
    }
    try {
      await chatApi.promoteChannelMessageToIssue(channelDetail.channel_id, message.message_id);
      if (threadRootId === message.message_id) {
        setThreadRootMessage((prev) =>
          prev
            ? {
                ...prev,
                surface: 'issue',
                display_status: 'active',
              }
            : prev,
        );
      }
      setSurfaceView('work');
      setWorkSurfaceFilter('issue');
      await loadChannel(channelDetail.channel_id, {
        display_kind: currentRootFilter.display_kind,
        display_status: currentRootFilter.display_status,
        preserveSelectedAgent: true,
      });
      addToast('success', bilingual('已升级为正式协作', 'Promoted to formal collaboration'));
    } catch (promoteError) {
      console.error('Failed to mark work thread highlighted:', promoteError);
      setError(bilingual('升级为正式协作失败，请稍后再试。', 'Failed to promote this item to formal collaboration. Please try again later.'));
    }
  }, [addToast, channelDetail, currentRootFilter.display_status, loadChannel, threadRootId]);

  const handleUpdateThreadStatus = useCallback(async (
    message: ChannelRenderMessage,
    status: ChatChannelDisplayStatus,
    successText: string,
  ) => {
    if (!channelDetail) return;
    try {
      await chatApi.updateChannelCollaborationStatus(channelDetail.channel_id, message.message_id, status);
      if (threadRootId === message.message_id) {
        setThreadRootMessage((prev) =>
          prev
            ? {
                ...prev,
                display_status: status,
              }
            : prev,
        );
      }
      await loadChannel(channelDetail.channel_id, {
        display_kind: currentRootFilter.display_kind,
        display_status: currentRootFilter.display_status,
        preserveSelectedAgent: true,
      });
      if (status === 'adopted') {
        addToast('success', `${successText}，可在 AI 产出中继续发布到团队文档`);
      } else {
        addToast('success', successText);
      }
    } catch (statusError) {
      console.error('Failed to update collaboration status:', statusError);
      setError(bilingual('更新协作项状态失败，请稍后再试。', 'Failed to update the collaboration status. Please try again later.'));
    }
  }, [addToast, channelDetail, currentRootFilter.display_kind, currentRootFilter.display_status, loadChannel, threadRootId]);

  const handleSyncThreadResult = useCallback(async (message: ChannelRenderMessage) => {
    if (!channelDetail) return;
    try {
      await chatApi.syncChannelCollaborationResult(channelDetail.channel_id, message.message_id);
      await loadChannel(channelDetail.channel_id, {
        display_kind: currentRootFilter.display_kind,
        display_status: currentRootFilter.display_status,
        preserveSelectedAgent: true,
      });
      setSurfaceView('update');
      addToast('success', '已同步结果到讨论区');
    } catch (syncError) {
      console.error('Failed to sync collaboration result:', syncError);
      setError(bilingual('同步结果到讨论区失败，请稍后再试。', 'Failed to sync the result back to the discussion area. Please try again later.'));
    }
  }, [addToast, channelDetail, currentRootFilter.display_kind, currentRootFilter.display_status, loadChannel]);

  const handleArchiveThread = useCallback(async (message: ChannelRenderMessage) => {
    void handleUpdateThreadStatus(message, 'rejected', bilingual('协作项已标记为未采用', 'Collaboration item marked as rejected'));
  }, [handleUpdateThreadStatus]);

  const openCollaborationWorkspace = useCallback(
    async (rootId: string, surface?: ChatChannelMessageSurface | null) => {
      if (!channelDetail) return;
      setSurfaceView('work');
      setWorkSurfaceFilter(surface === 'issue' ? 'issue' : 'temporary');
      setWorkStatusFilter('all');
      setDesktopThreadMode(true);
      setThreadRootExpanded(false);
      if (isMobile) {
        setSidePanelMode('thread');
      }
      await loadThread(channelDetail.channel_id, rootId);
    },
    [channelDetail, isMobile, loadThread],
  );

  const handleStartCollaborationFromCard = useCallback(
    async (message: ChannelRenderMessage) => {
      if (!channelDetail || sending) return;
      const linkedCollaborationId =
        typeof message.metadata?.linked_collaboration_id === 'string'
          ? message.metadata.linked_collaboration_id
          : null;
      if (linkedCollaborationId) {
        const linkedSurface =
          collaborationRootMessages.find((item) => item.message_id === linkedCollaborationId)?.surface
          || null;
        await openCollaborationWorkspace(linkedCollaborationId, linkedSurface);
        return;
      }

      const sourceMessageId =
        typeof message.metadata?.source_message_id === 'string'
          ? message.metadata.source_message_id
          : null;
      const linkedMessageIds = Array.isArray(message.metadata?.linked_message_ids)
        ? message.metadata.linked_message_ids.filter((item): item is string => typeof item === 'string')
        : [];

      const sourceMessage = sourceMessageId
        ? mainChannelMessages.find((item) => item.message_id === sourceMessageId)
        : null;
      const linkedMessages = linkedMessageIds.length > 0
        ? mainChannelMessages.filter((item) => linkedMessageIds.includes(item.message_id))
        : [];

      const derivedContent = (
        sourceMessage?.content_text?.trim()
        || linkedMessages.map((item) => item.content_text?.trim()).filter(Boolean).join('\n')
        || (typeof message.metadata?.summary_text === 'string' ? message.metadata.summary_text : '')
        || message.content_text
      ).trim();

      if (!derivedContent) {
      setError(bilingual('当前卡片没有可继续推进的内容。', 'There is nothing actionable to continue from this card.'));
        return;
      }

      setSending(true);
      try {
        const response = await chatApi.sendChannelMessage(channelDetail.channel_id, {
          content: derivedContent,
          surface: 'temporary',
          agent_id: message.agent_id || channelDetail.default_agent_id,
          attached_document_ids: sourceMessage ? getAttachedDocumentIds(sourceMessage) : [],
          mentions: [],
        });
        await loadChannel(channelDetail.channel_id, {
          display_kind: currentRootFilter.display_kind,
          display_status: currentRootFilter.display_status,
          preserveSelectedAgent: true,
        });
        await openCollaborationWorkspace(response.root_message_id, 'temporary');
        if (response.streaming) {
          openStream(channelDetail.channel_id, response.root_message_id, 'temporary', 'active');
        }
        addToast('success', '已从讨论区卡片创建协作项');
      } catch (startError) {
        console.error('Failed to start collaboration from card:', startError);
      setError(bilingual('开始协作失败，请稍后再试。', 'Failed to start collaboration. Please try again later.'));
      } finally {
        setSending(false);
      }
    },
    [
      addToast,
      channelDetail,
      collaborationRootMessages,
      currentRootFilter.display_kind,
      currentRootFilter.display_status,
      loadChannel,
      mainChannelMessages,
      openCollaborationWorkspace,
      sending,
    ],
  );

  const openOnboardingForm = useCallback(() => {
    if (!channelDetail) return;
    setForm((prev) => ({
      ...prev,
      channelGoal: channelDetail.orchestrator_state?.channel_goal || '',
      participantNotes: channelDetail.orchestrator_state?.participant_notes || '',
      expectedOutputs: channelDetail.orchestrator_state?.expected_outputs || '',
      collaborationStyle: channelDetail.orchestrator_state?.collaboration_style || '',
    }));
    setOnboardingOpen(true);
  }, [channelDetail]);

  const handleSaveOnboarding = useCallback(async () => {
    if (!channelDetail) return;
    try {
      const updated = await chatApi.updateChannel(channelDetail.channel_id, {
        channel_goal: form.channelGoal || null,
        participant_notes: form.participantNotes || null,
        expected_outputs: form.expectedOutputs || null,
        collaboration_style: form.collaborationStyle || null,
      });
      setOnboardingOpen(false);
      setChannelDetail(updated);
      setChannels((prev) =>
        prev.map((item) => (item.channel_id === updated.channel_id ? updated : item)),
      );
      addToast('success', bilingual('已写入频道目标、参与人、产出和协作方式', 'Channel goal, participants, outputs, and collaboration style saved'));
    } catch (saveError) {
      console.error('Failed to save onboarding fields:', saveError);
      setError(bilingual('保存频道启动信息失败，请稍后再试。', 'Failed to save the channel kickoff details. Please try again later.'));
    }
  }, [addToast, channelDetail, form.channelGoal, form.collaborationStyle, form.expectedOutputs, form.participantNotes]);

  const handleRejectSuggestionCard = useCallback(
    async (message: ChannelRenderMessage) => {
      if (!channelDetail) return;
      const linkedCollaborationId =
        typeof message.metadata?.linked_collaboration_id === 'string'
          ? message.metadata.linked_collaboration_id
          : null;

      try {
        if (linkedCollaborationId) {
          await chatApi.updateChannelCollaborationStatus(
            channelDetail.channel_id,
            linkedCollaborationId,
            'rejected',
          );
          if (threadRootId === linkedCollaborationId) {
            closeThreadPanel();
          }
        }

        await chatApi.updateChannelCollaborationStatus(
          channelDetail.channel_id,
          message.message_id,
          'rejected',
        );

        await loadChannel(channelDetail.channel_id, {
          display_kind: currentRootFilter.display_kind,
          display_status: currentRootFilter.display_status,
          preserveSelectedAgent: true,
        });

        addToast(
          'success',
          linkedCollaborationId
        ? bilingual('已拒绝建议，关联协作项已移入拒绝分类', 'Suggestion rejected. Linked collaboration items were also moved to rejected.')
        : bilingual('已拒绝建议，已移入拒绝分类', 'Suggestion rejected and moved to rejected.'),
        );
      } catch (rejectError) {
        console.error('Failed to reject suggestion card:', rejectError);
      setError(bilingual('拒绝建议失败，请稍后再试。', 'Failed to reject the suggestion. Please try again later.'));
      }
    },
    [
      addToast,
      channelDetail,
      closeThreadPanel,
      currentRootFilter.display_kind,
      currentRootFilter.display_status,
      loadChannel,
      threadRootId,
    ],
  );

  const requestMarkThreadHighlightedConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('升级为正式协作', 'Promote to formal collaboration'),
      description: bilingual('把这条临时协作升级为正式协作。', 'Promote this temporary collaboration item into a formal collaboration item.'),
      confirmText: bilingual('确认升级', 'Confirm promotion'),
      actionLabel: bilingual('升级为正式协作', 'Promote'),
      subject,
      note: bilingual('升级后它会进入正式协作分类，作为需要持续推进的重点工作。', 'After promotion it will move into formal collaboration as work that should continue to be driven forward.'),
      onConfirm: () => handleMarkThreadHighlighted(message),
    });
  }, [handleMarkThreadHighlighted, requestCollaborationActionConfirm]);

  const requestArchiveThreadConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('标记未采用', 'Mark as rejected'),
      description: bilingual('把这条协作项移入未采用分类。', 'Move this collaboration item into the rejected state.'),
      confirmText: bilingual('确认未采用', 'Confirm rejection'),
      actionLabel: bilingual('未采用', 'Rejected'),
      subject,
      note: bilingual('这不会删除内容，只是把它从当前推进流转到未采用分类。', 'This does not delete the content. It only moves the item out of the active flow into rejected.'),
      variant: 'destructive',
      onConfirm: () => handleArchiveThread(message),
    });
  }, [handleArchiveThread, requestCollaborationActionConfirm]);

  const requestAwaitingConfirmationConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('标记等判断', 'Mark as needs your decision'),
      description: bilingual('把这条协作项交回你来判断下一步。', 'Hand this collaboration item back to you for the next decision.'),
      confirmText: bilingual('确认标记', 'Confirm'),
      actionLabel: bilingual('等判断', 'Needs decision'),
      subject,
      note: bilingual('适合当前需要人工拍板、决定采纳或继续推进的情况。', 'Use this when human judgment is needed to decide whether to adopt or continue.'),
      onConfirm: () =>
        handleUpdateThreadStatus(message, 'awaiting_confirmation', bilingual('已标记为等你判断', 'Marked as needs your decision')),
    });
  }, [handleUpdateThreadStatus, requestCollaborationActionConfirm]);

  const requestAdoptThreadConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('标记采用', 'Mark as adopted'),
      description: bilingual('把这条协作结果正式标记为已采用。', 'Mark this collaboration result as formally adopted.'),
      confirmText: bilingual('确认采用', 'Confirm adoption'),
      actionLabel: bilingual('已采用', 'Adopted'),
      subject,
      note: bilingual('采用后可继续在 AI 产出中整理并发布到团队文档。', 'After adoption you can refine it in AI outputs and publish it to team docs.'),
      onConfirm: () =>
        handleUpdateThreadStatus(message, 'adopted', bilingual('协作结果已标记为已采用', 'Collaboration result marked as adopted')),
    });
  }, [handleUpdateThreadStatus, requestCollaborationActionConfirm]);

  const requestSyncThreadResultConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('同步到讨论', 'Sync back to discussion'),
      description: bilingual('把当前结果同步回讨论区，便于团队成员查看。', 'Sync the current result back to the discussion area so the team can review it.'),
      confirmText: bilingual('确认同步', 'Confirm sync'),
      actionLabel: bilingual('同步到讨论', 'Sync to discussion'),
      subject,
      note: bilingual('同步后会在讨论区生成结果卡，方便团队继续跟进。', 'A result card will be generated in the discussion area so the team can continue following up.'),
      onConfirm: () => handleSyncThreadResult(message),
    });
  }, [handleSyncThreadResult, requestCollaborationActionConfirm]);

  const requestStartCollaborationConfirm = useCallback((message: ChannelRenderMessage) => {
    if (message.display_kind === 'onboarding' || message.metadata?.card_purpose === 'channel_onboarding') {
      openOnboardingForm();
      return;
    }
    const linkedCollaborationId =
      typeof message.metadata?.linked_collaboration_id === 'string'
        ? message.metadata.linked_collaboration_id
        : null;
    if (linkedCollaborationId) {
      void handleStartCollaborationFromCard(message);
      return;
    }
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('开始协作', 'Start collaboration'),
      description: bilingual('基于这张建议卡创建一条新的协作项。', 'Create a new collaboration item from this suggestion card.'),
      confirmText: bilingual('确认开始', 'Confirm start'),
      actionLabel: bilingual('开始协作', 'Start collaboration'),
      subject,
      note: bilingual('创建后会直接进入协作模式，围绕这件事继续推进。', 'Once created, it will enter collaboration mode and continue advancing this work.'),
      onConfirm: () => handleStartCollaborationFromCard(message),
    });
  }, [handleStartCollaborationFromCard, openOnboardingForm, requestCollaborationActionConfirm]);

  const requestRejectSuggestionConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: bilingual('拒绝建议', 'Reject suggestion'),
      description: bilingual('把这张建议卡移入拒绝分类。', 'Move this suggestion card into the rejected state.'),
      confirmText: bilingual('确认拒绝', 'Confirm rejection'),
      actionLabel: bilingual('拒绝建议', 'Reject suggestion'),
      subject,
      note: bilingual('如果它关联了协作项，关联协作项也会一起进入拒绝分类。', 'If it is linked to collaboration items, those linked items will also move into rejected.'),
      variant: 'destructive',
      onConfirm: () => handleRejectSuggestionCard(message),
    });
  }, [handleRejectSuggestionCard, requestCollaborationActionConfirm]);

  const handleSend = useCallback(
    async (
      threadTarget?: string | null,
      submitMode?: 'discussion' | 'work',
    ) => {
      if (!channelDetail) return;
      const rawComposerText = threadTarget ? threadComposeText : composeText;
      const content = rawComposerText.trim();
      if (!content || sending) return;
      const selectedRefsSnapshot = [...selectedCapabilityRefs];
      const pendingDocIdsSnapshot = [...pendingDocIds];
      const finalContent = buildCapabilityDraft(selectedRefsSnapshot, content);
      const parsedMentions = deriveMentionsFromText(rawComposerText, mentionOptions);
      const agentMentions = parsedMentions.filter((item) => item.mention_type === 'agent');
      if (!threadTarget && agentMentions.length > 1) {
        const message = bilingual('一条讨论消息只能 @ 一个 Agent。请只保留一个 Agent 后再发送。', 'A discussion message can mention only one agent. Keep just one agent mention before sending.');
        setError(message);
        addToast('error', message);
        return;
      }
      const currentMember =
        teamMembers.find((item) => item.userId === user?.id) || null;
      const mentionedAgent =
        !threadTarget && agentMentions.length === 1
          ? visibleAgents.find((agent) => agent.id === agentMentions[0].target_id) || null
          : null;
      const collaborationAgent =
        mentionedAgent ||
        visibleAgents.find((agent) => agent.id === resolvedComposerAgentId) ||
        visibleAgents.find((agent) => agent.id === channelDetail.default_agent_id) ||
        null;
      const selectedAgent = collaborationAgent;
      const rootMentions = [...parsedMentions];
      const explicitSubmitMode = submitMode;
      const hasExplicitAiIntent = Boolean(
        selectedRefsSnapshot.length ||
        pendingDocIdsSnapshot.length ||
        (selectedAgentId && selectedAgentId !== DEFAULT_CHANNEL_AGENT_VALUE) ||
        surfaceView === 'work',
      );
      const wantsTemporaryMentionThread = !threadTarget && agentMentions.length === 1;
      const wantsWorkAtRoot = explicitSubmitMode
        ? explicitSubmitMode === 'work' || wantsTemporaryMentionThread
        : hasExplicitAiIntent || wantsTemporaryMentionThread;
      const rootSurface: ChatChannelMessageSurface = threadTarget
        ? (threadRootMessage?.surface || 'temporary')
        : wantsWorkAtRoot
          ? 'temporary'
          : 'activity';
      const requiresAgentReply = threadTarget ? true : rootSurface !== 'activity';
      const optimisticUser = buildOptimisticUserMessage(
        channelDetail.channel_id,
        currentMember,
        finalContent,
        rootSurface,
        rootMentions,
        threadTarget,
        threadTarget || null,
      );
      const optimisticAssistant = buildStreamingAssistantMessage(
        channelDetail.channel_id,
        normalizeAgentDisplayName(selectedAgent?.name || channelDetail.default_agent_name),
        rootSurface,
        threadTarget,
        optimisticUser.message_id,
      );
      if (threadTarget) {
        stickThreadToBottomRef.current = true;
      } else {
        stickMainToBottomRef.current = true;
      }
      const shouldEnterCollaborationWorkspace = !threadTarget && requiresAgentReply;
      if (shouldEnterCollaborationWorkspace) {
        setSurfaceView('work');
        setWorkSurfaceFilter(rootSurface === 'issue' ? 'issue' : 'temporary');
        setWorkStatusFilter('all');
        setDesktopThreadMode(true);
        if (isMobile) {
          setSidePanelMode('thread');
        }
        setThreadRootId(optimisticUser.message_id);
        setThreadRootMessage(optimisticUser);
        setThreadMessages([
          {
            ...optimisticAssistant,
            thread_root_id: optimisticUser.message_id,
          },
        ]);
        activeThreadRootIdRef.current = optimisticUser.message_id;
        stickThreadToBottomRef.current = true;
      }
      if (threadTarget) {
        setThreadMessages((prev) => [...prev, optimisticUser, optimisticAssistant]);
        setThreadComposeText('');
      } else if (requiresAgentReply) {
        setMessages((prev) => [...prev, optimisticUser, optimisticAssistant]);
        setComposeText('');
      } else {
        setSurfaceView('update');
        setMessages((prev) => [...prev, optimisticUser]);
        setComposeText('');
      }
      setSending(true);
      try {
        if (threadTarget) {
          await chatApi.sendChannelThreadMessage(channelDetail.channel_id, threadTarget, {
            content: finalContent,
            agent_id: resolvedComposerAgentId || channelDetail.default_agent_id,
            // Thread replies currently target the root thread, not arbitrary nested messages.
            // Sending a local optimistic id here makes the backend look up a message that doesn't exist yet.
            parent_message_id: threadTarget,
            attached_document_ids: pendingDocIdsSnapshot,
            mentions: rootMentions,
          });
        } else {
          const response = await chatApi.sendChannelMessage(channelDetail.channel_id, {
            content: finalContent,
            surface: rootSurface,
            agent_id: requiresAgentReply ? (collaborationAgent?.id || channelDetail.default_agent_id) : null,
            parent_message_id: null,
            attached_document_ids: pendingDocIdsSnapshot,
            mentions: rootMentions,
          });
          setAttachedDocs([]);
          setPendingDocIds([]);
          setSelectedCapabilityRefs([]);
          if (!requiresAgentReply || !response.streaming) {
            await loadChannel(channelDetail.channel_id, {
              silent: true,
              preserveSelectedAgent: true,
              display_kind: (rootSurface === 'activity' ? 'discussion' : 'collaboration') as ChatChannelDisplayKind,
              display_status:
                rootSurface === 'issue' || rootSurface === 'temporary'
                  ? 'active'
                    : undefined,
            });
            if (isMobile) {
              const nextView = rootSurface === 'activity' ? 'update' : 'work';
              if (surfaceView !== nextView) {
                setSurfaceView(nextView as ChannelDisplayView);
              }
            }
            if (rootSurface !== 'activity') {
              await openCollaborationWorkspace(response.root_message_id, rootSurface);
            }
            setSending(false);
            return;
          }
          if (isMobile && surfaceView !== 'work') {
            setSurfaceView('work');
          }
          if (isMobile) {
            setSidePanelMode('thread');
          }
          setThreadRootId(response.root_message_id);
          setThreadRootMessage({
            ...optimisticUser,
            message_id: response.root_message_id,
          });
          setThreadMessages([
            {
              ...optimisticAssistant,
              thread_root_id: response.root_message_id,
            },
          ]);
          activeThreadRootIdRef.current = response.root_message_id;
          stickThreadToBottomRef.current = true;
          openStream(channelDetail.channel_id, response.root_message_id, rootSurface, 'active');
          return;
        }
        setAttachedDocs([]);
        setPendingDocIds([]);
        setSelectedCapabilityRefs([]);
        openStream(
          channelDetail.channel_id,
          threadTarget,
          threadRootMessage?.surface || 'temporary',
          threadRootMessage?.thread_state || 'active',
        );
      } catch (sendError) {
        console.error('Failed to send channel message:', sendError);
        setSending(false);
        setError(bilingual('发送频道消息失败，请稍后再试。', 'Failed to send the channel message. Please try again later.'));
        if (threadTarget) {
          await loadThread(channelDetail.channel_id, threadTarget);
        } else {
          await loadChannel(channelDetail.channel_id, {
            display_kind: currentRootFilter.display_kind,
            display_status: currentRootFilter.display_status,
          });
        }
      }
    },
    [
      addToast,
      channelDetail,
      composeText,
      loadChannel,
      loadThread,
      members,
      openStream,
      resolvedComposerAgentId,
      selectedAgentId,
      isMobile,
      surfaceView,
      sending,
      pendingDocIds,
      selectedCapabilityRefs,
      teamMembers,
      threadRootMessage?.surface,
      threadComposeText,
      visibleAgents,
    ],
  );

  useEffect(() => {
    if (!channelDetail) {
      resetCreateForm();
      return;
    }
    setForm({
        name: channelDetail.name,
        description: channelDetail.description || '',
        visibility: channelDetail.visibility,
        channelType: channelDetail.channel_type || 'general',
        defaultAgentId: channelDetail.default_agent_id,
      workspaceDisplayName: channelDetail.workspace_display_name || channelDetail.name,
      repoDefaultBranch: channelDetail.repo_default_branch || 'main',
      agentAutonomyMode: channelDetail.orchestrator_state?.agent_autonomy_mode || 'standard',
      channelGoal: channelDetail.orchestrator_state?.channel_goal || '',
      participantNotes: channelDetail.orchestrator_state?.participant_notes || '',
      expectedOutputs: channelDetail.orchestrator_state?.expected_outputs || '',
      collaborationStyle: channelDetail.orchestrator_state?.collaboration_style || '',
      memberUserIds: members.map((item) => item.user_id),
    });
  }, [channelDetail, members, resetCreateForm]);

  const memberOptions = useMemo(
    () =>
      teamMembers.filter(
        (member) => !members.some((existing) => existing.user_id === member.userId),
      ),
    [members, teamMembers],
  );

  const openChannelDocuments = useCallback((folderPath?: string | null) => {
    if (!folderPath) return;
    try {
      window.localStorage.setItem(`agime.documents.${teamId}.recentFolder`, folderPath);
    } catch {
      // ignore local storage failures
    }
    navigate(`/teams/${teamId}?section=documents`);
  }, [navigate, teamId]);

  const handleUploadToChannelFolder = useCallback(
    async (files: FileList | null) => {
      if (!files || !channelDetail?.document_folder_path || uploadingDocument) {
        return;
      }
      setUploadingDocument(true);
      setError(null);
      try {
        const uploadedDocs: DocumentSummary[] = [];
        for (const file of Array.from(files)) {
          const doc = await documentApi.uploadDocument(
            teamId,
            file,
            channelDetail.document_folder_path,
          );
          uploadedDocs.push(doc);
        }
        addToast('success', `已上传到 ${channelDetail.document_folder_path}`);
        if (uploadedDocs.length > 0) {
          setAttachedDocs((prev) => {
            const existingIds = new Set(prev.map((item) => item.id));
            return [...prev, ...uploadedDocs.filter((item) => !existingIds.has(item.id))];
          });
          setPendingDocIds((prev) => {
            const existingIds = new Set(prev);
            return [
              ...prev,
              ...uploadedDocs.map((item) => item.id).filter((id) => !existingIds.has(id)),
            ];
          });
        }
        if (channelDetail.document_folder_path) {
          const docs = await documentApi.listDocuments(
            teamId,
            1,
            8,
            channelDetail.document_folder_path,
          );
          setChannelDocuments(docs.items);
        }
      } catch (uploadError) {
        console.error('Failed to upload channel documents:', uploadError);
        setError(bilingual('上传到频道文档目录失败，请稍后再试。', 'Failed to upload into the channel document directory. Please try again later.'));
      } finally {
        setUploadingDocument(false);
        if (fileInputRef.current) {
          fileInputRef.current.value = '';
        }
      }
    },
    [addToast, channelDetail?.document_folder_path, teamId, uploadingDocument],
  );

  const applyCapabilitySelection = useCallback((items: ChatCapabilitySelection[]) => {
    const nextRefs = Array.from(
      new Set([
        ...selectedCapabilityRefs,
        ...items.map((item) => item.ref).filter((value) => value.trim().length > 0),
      ]),
    );
    setSelectedCapabilityRefs(nextRefs);
    setCapabilityDetailKey(null);
    setShowCapabilityPicker(false);
    setComposerToolsOpen(false);
  }, [selectedCapabilityRefs]);

  const removeCapabilityRef = useCallback((ref: string) => {
    setSelectedCapabilityRefs((prev) => prev.filter((item) => item !== ref));
  }, []);

  const attachDocsToComposer = useCallback((docs: DocumentSummary[]) => {
    if (!docs.length) return;
    setAttachedDocs((prev) => {
      const existingIds = new Set(prev.map((item) => item.id));
      return [...prev, ...docs.filter((item) => !existingIds.has(item.id))];
    });
    setPendingDocIds((prev) => {
      const existingIds = new Set(prev);
      return [...prev, ...docs.map((item) => item.id).filter((id) => !existingIds.has(id))];
    });
  }, []);

  const refreshChannelDocumentViews = useCallback(async () => {
    if (!channelDetail) return;
    if (channelDetail.document_folder_path) {
      setLoadingChannelDocuments(true);
    }
    setLoadingChannelAiOutputs(true);
    try {
      const [docs, aiOutputs] = await Promise.all([
        channelDetail.document_folder_path
          ? documentApi.listDocuments(teamId, 1, 8, channelDetail.document_folder_path)
          : Promise.resolve({ items: [], total: 0, page: 1, limit: 8, total_pages: 0 }),
        documentApi.listAiWorkbench(teamId, {
          sourceSpaceType: 'team_channel',
          sourceChannelId: channelDetail.channel_id,
          page: 1,
          limit: 8,
        }),
      ]);
      setChannelDocuments(docs.items);
      setChannelAiOutputs(
        aiOutputs.items.filter((doc) => {
          if (channelDetail.document_folder_path && doc.folder_path === channelDetail.document_folder_path) {
            return false;
          }
          if (doc.status === 'accepted' || doc.status === 'archived' || doc.status === 'superseded') {
            return false;
          }
          return true;
        }),
      );
    } catch (error) {
      console.error('Failed to refresh channel document views:', error);
    } finally {
      setLoadingChannelDocuments(false);
      setLoadingChannelAiOutputs(false);
    }
  }, [channelDetail, teamId]);

  const loadFolders = useCallback(async () => {
    setFoldersLoading(true);
    try {
      const tree = await folderApi.getFolderTree(teamId);
      setFolderTree(tree || []);
    } catch (error) {
      console.error('Failed to load folder tree for publish dialog:', error);
      setFolderTree([]);
    } finally {
      setFoldersLoading(false);
    }
  }, [teamId]);

  const handlePromoteAiOutputToChannelDocs = useCallback(async (doc: DocumentSummary) => {
    if (!channelDetail?.document_folder_path || promotingDocId) return;
    setPromotingDocId(doc.id);
    try {
      await documentApi.updateDocument(teamId, doc.id, {
        folder_path: channelDetail.document_folder_path,
      });
      await documentApi.updateStatus(teamId, doc.id, 'accepted');
      addToast('success', '已加入频道资料');
      await refreshChannelDocumentViews();
    } catch (error) {
      console.error('Failed to promote AI output to channel docs:', error);
      setError(bilingual('加入频道资料失败，请稍后再试。', 'Failed to add this item to channel documents. Please try again later.'));
    } finally {
      setPromotingDocId(null);
    }
  }, [addToast, channelDetail, promotingDocId, refreshChannelDocumentViews, teamId]);

  const openPublishDialog = useCallback(async (doc: DocumentSummary) => {
    setPublishTargetDoc(doc);
    setPublishName(doc.display_name || doc.name);
    setPublishFolderPath(doc.folder_path || '/');
    setPublishDialogOpen(true);
    await loadFolders();
  }, [loadFolders]);

  const closePublishDialog = useCallback(() => {
    setPublishDialogOpen(false);
    setPublishTargetDoc(null);
    setPublishName('');
    setPublishFolderPath('/');
  }, []);

  const confirmPublishAiOutput = useCallback(async () => {
    if (!publishTargetDoc || publishingDocId) return;
    setPublishingDocId(publishTargetDoc.id);
    try {
      const updates: {
        display_name?: string;
        folder_path?: string;
      } = {};
      const nextDisplayName = publishName.trim();
      const currentDisplayName = publishTargetDoc.display_name || publishTargetDoc.name;
      if (nextDisplayName && nextDisplayName !== currentDisplayName) {
        updates.display_name = nextDisplayName;
      }
      const nextFolderPath = publishFolderPath || '/';
      const currentFolderPath = publishTargetDoc.folder_path || '/';
      if (nextFolderPath !== currentFolderPath) {
        updates.folder_path = nextFolderPath;
      }
      if (Object.keys(updates).length > 0) {
        await documentApi.updateDocument(teamId, publishTargetDoc.id, updates);
      }
      await documentApi.updateStatus(teamId, publishTargetDoc.id, 'accepted');
      addToast('success', '已发布到团队文档');
      closePublishDialog();
      await refreshChannelDocumentViews();
    } catch (error) {
      console.error('Failed to publish AI output to team docs:', error);
      setError(bilingual('发布到团队文档失败，请稍后再试。', 'Failed to publish to team documents. Please try again later.'));
    } finally {
      setPublishingDocId(null);
    }
  }, [
    addToast,
    closePublishDialog,
    publishFolderPath,
    publishName,
    publishTargetDoc,
    publishingDocId,
    refreshChannelDocumentViews,
    teamId,
  ]);

  const renderMessage = useCallback(
    (message: ChannelRenderMessage, groupedWithPrevious = false) => {
      const rootId = message.thread_root_id || message.message_id;
      const threadDocumentCount = threadDocumentCountMap.get(rootId) || 0;
      const threadAiOutputCount = threadAiOutputCountMap.get(rootId) || 0;
      const threadPreviewText = pairedAssistantByRootId.get(rootId)?.content_text || null;
      const showThreadSummary = message.display_kind === 'collaboration';
      if (message.author_type === 'system') {
        return <ChannelSystemBubble key={message.message_id} message={message} />;
      }
      if (message.display_kind === 'discussion') {
        return (
          <ChannelActivityBubble
            key={message.message_id}
            message={message}
            isOwn={message.author_user_id === user?.id}
            groupedWithPrevious={groupedWithPrevious}
          />
        );
      }
      if (
        message.author_type === 'agent'
        || message.display_kind === 'onboarding'
        || message.display_kind === 'suggestion'
        || message.display_kind === 'result'
      ) {
          const linkedCollaborationId =
            typeof message.metadata?.linked_collaboration_id === 'string'
              ? message.metadata.linked_collaboration_id
              : null;
          const cardPurpose = message.metadata?.card_purpose as string | undefined;
          const isOnboardingCard =
            message.display_kind === 'onboarding'
            || cardPurpose === 'channel_onboarding';
          const primaryActionLabel = isOnboardingCard
            ? bilingual('填写信息', 'Fill in details')
            : linkedCollaborationId
              ? bilingual('查看协作项', 'View collaboration item')
              : cardPurpose === 'discussion_summary' || message.display_kind === 'suggestion'
                ? bilingual('开始协作', 'Start collaboration')
                : null;
          const secondaryActionLabel = isOnboardingCard
            ? null
            : cardPurpose === 'discussion_summary' || message.display_kind === 'suggestion'
              ? bilingual('快速拒绝', 'Quick reject')
              : null;
          return (
            <ChannelAssistantBubble
              key={message.message_id}
              message={message}
              threadDocumentCount={threadDocumentCount}
              threadAiOutputCount={threadAiOutputCount}
              threadPreviewText={threadPreviewText}
              onOpenThread={handleOpenThread}
              surface={message.surface}
              showThreadSummary={showThreadSummary}
              groupedWithPrevious={groupedWithPrevious}
              onPromote={message.surface === 'temporary' ? () => requestMarkThreadHighlightedConfirm(message) : undefined}
              onArchive={message.surface === 'temporary' || message.surface === 'issue' ? () => requestArchiveThreadConfirm(message) : undefined}
              primaryActionLabel={primaryActionLabel}
              onPrimaryAction={
                primaryActionLabel
                  ? () => requestStartCollaborationConfirm(message)
                  : undefined
              }
              secondaryActionLabel={secondaryActionLabel}
              onSecondaryAction={
                secondaryActionLabel
                  ? () => requestRejectSuggestionConfirm(message)
                  : undefined
              }
            />
          );
        }
      return (
        <ChannelUserBubble
          key={message.message_id}
          message={message}
          isOwn={message.author_user_id === user?.id}
          threadDocumentCount={threadDocumentCount}
          threadAiOutputCount={threadAiOutputCount}
          threadPreviewText={threadPreviewText}
          onOpenThread={handleOpenThread}
          surface={message.surface}
          showThreadSummary={showThreadSummary}
          groupedWithPrevious={groupedWithPrevious}
          onPromote={message.surface === 'temporary' ? () => requestMarkThreadHighlightedConfirm(message) : undefined}
          onArchive={message.surface === 'temporary' || message.surface === 'issue' ? () => requestArchiveThreadConfirm(message) : undefined}
        />
      );
    },
    [handleOpenThread, pairedAssistantByRootId, requestArchiveThreadConfirm, requestMarkThreadHighlightedConfirm, requestRejectSuggestionConfirm, requestStartCollaborationConfirm, threadAiOutputCountMap, threadDocumentCountMap, user?.id],
  );

  const renderDocumentsInspector = () => {
    if (!channelDetail) return null;
    return (
      <div className="collab-inspector-pane">
        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('频道资料', 'Channel documents')}</div>
                <span className="collab-inspector-count">{channelDocuments.length}</span>
              </div>
              <div className="collab-inspector-section-meta">
                {bilingual('已上传到频道目录的资料，需要按需附加到当前消息或线程。', 'Documents uploaded into the channel directory can be attached to the current message or thread when needed.')}
              </div>
            </div>
          </div>
          <div className="collab-inspector-meta-rail">
            <span className="collab-inspector-meta-label">{bilingual('目录', 'Directory')}</span>
            <span
              className="collab-inspector-inline-meta collab-inspector-mono"
              title={channelDetail.document_folder_path || '/'}
            >
              {channelDetail.document_folder_path || '/'}
            </span>
          </div>
          <div className="collab-inspector-toolbar">
            {channelDetail.document_folder_path ? (
              <Button
                variant="ghost"
                size="sm"
                className="h-8 rounded-[10px] px-2.5 text-[12px]"
                onClick={() => openChannelDocuments(channelDetail.document_folder_path)}
              >
                {bilingual('打开文档', 'Open docs')}
              </Button>
            ) : null}
            <Button
              variant="outline"
              size="sm"
              className="h-8 rounded-[10px] px-2.5 text-[12px]"
              onClick={() => fileInputRef.current?.click()}
              disabled={uploadingDocument}
            >
              {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传资料', 'Upload docs')}
            </Button>
          </div>
        </section>
        <div className="collab-inspector-list">
          {loadingChannelDocuments ? (
            <div className="collab-inspector-empty">{bilingual('正在读取频道资料…', 'Loading channel documents…')}</div>
          ) : channelDocuments.length > 0 ? (
            <div className="space-y-2.5">
              {channelDocuments.map((doc) => (
                <div
                  key={doc.id}
                  className="collab-inspector-item collab-inspector-item-stack"
                >
                  <div className="min-w-0">
                    <div className="collab-inspector-item-title truncate">
                      {doc.display_name || doc.name}
                    </div>
                    <div className="collab-inspector-item-meta collab-inspector-mono" title={doc.folder_path || '/'}>
                      {doc.folder_path || '/'}
                    </div>
                  </div>
                  <div className="collab-inspector-item-actions">
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-8 rounded-[10px] px-2.5 text-[12px]"
                      onClick={() => attachDocsToComposer([doc])}
                    >
                      {bilingual('附加', 'Attach')}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 rounded-[10px] px-2.5 text-[12px]"
                      onClick={() => openChannelDocuments(channelDetail.document_folder_path)}
                    >
                      {bilingual('打开', 'Open')}
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="collab-inspector-empty">{bilingual('这个频道还没有资料。先上传到频道目录，再按需附加到当前消息。', 'This channel has no documents yet. Upload them into the channel directory first, then attach them when needed.')}</div>
          )}
        </div>
      </div>
    );
  };

  const renderAiOutputsInspector = () => {
    if (!channelDetail) return null;
    return (
      <div className="collab-inspector-pane">
        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('AI 产出', 'AI outputs')}</div>
                <span className="collab-inspector-count">{channelAiOutputs.length}</span>
              </div>
              <div className="collab-inspector-section-meta">
                {bilingual('频道里的草稿、总结和结果。先附加使用，再决定是否加入频道资料或发布。', 'Drafts, summaries, and results generated in this channel. Attach them first, then decide whether to add them to channel docs or publish them.')}
              </div>
            </div>
          </div>
        </section>
        <div className="collab-inspector-list">
        {loadingChannelAiOutputs ? (
          <div className="collab-inspector-empty">{bilingual('正在读取 AI 产出…', 'Loading AI outputs…')}</div>
        ) : channelAiOutputs.length > 0 ? (
          <div className="space-y-2.5">
            {channelAiOutputs.map((doc) => (
              <div
                key={doc.id}
                className="collab-inspector-item collab-inspector-item-stack"
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <div className="collab-inspector-item-title truncate">
                        {doc.display_name || doc.name}
                      </div>
                      {doc.ai_workbench_group ? (
                        <span className="collab-inspector-inline-meta">
                          {renderAiWorkbenchGroupLabel(doc.ai_workbench_group)}
                        </span>
                      ) : null}
                    </div>
                    <div className="collab-inspector-item-meta">
                      {doc.source_thread_root_id ? bilingual('来自当前频道某条线程', 'From a thread in this channel') : bilingual('来自当前频道', 'From this channel')}
                    </div>
                  </div>
                  {doc.source_thread_root_id ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 rounded-[10px] px-2.5 text-[12px]"
                      onClick={() => void loadThread(channelDetail.channel_id, doc.source_thread_root_id!)}
                    >
                      {bilingual('来源线程', 'Source thread')}
                    </Button>
                    ) : null}
                </div>
                <div className="collab-inspector-item-actions">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[10px] px-2.5 text-[12px]"
                    onClick={() => attachDocsToComposer([doc])}
                  >
                    {bilingual('附加', 'Attach')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[10px] px-2.5 text-[12px]"
                    onClick={() => void handlePromoteAiOutputToChannelDocs(doc)}
                    disabled={promotingDocId === doc.id || doc.folder_path === channelDetail.document_folder_path}
                  >
                    {doc.folder_path === channelDetail.document_folder_path
                      ? bilingual('已在频道资料', 'Already in channel docs')
                      : promotingDocId === doc.id
                        ? bilingual('处理中…', 'Processing…')
                        : bilingual('加入频道资料', 'Add to channel docs')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[10px] px-2.5 text-[12px]"
                    onClick={() => void openPublishDialog(doc)}
                    disabled={publishingDocId === doc.id || doc.status === 'accepted'}
                  >
                    {doc.status === 'accepted' ? bilingual('已发布', 'Published') : bilingual('发布到团队文档', 'Publish to team docs')}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="collab-inspector-empty">{bilingual('当前频道还没有 AI 产出。让 Agent 生成草稿、总结或报告后，这里会自动出现。', 'There are no AI outputs in this channel yet. After the agent generates drafts, summaries, or reports, they will appear here automatically.')}</div>
        )}
      </div>
    </div>
    );
  };

  const renderMembersContent = () => (
    <div className="collab-inspector-pane">
      <section className="collab-inspector-section">
        <div className="collab-inspector-section-head">
          <div className="min-w-0 flex-1">
            <div className="collab-inspector-section-header-row">
              <div className="collab-inspector-section-title">{bilingual('成员管理', 'Member management')}</div>
              <span className="collab-inspector-count">{members.length}</span>
            </div>
            <div className="collab-inspector-section-meta">{bilingual('公开频道默认可见；私密频道需要手动维护成员与角色。', 'Public channels are visible by default. Private channels require manual member and role management.')}</div>
          </div>
        </div>
        <div className="collab-inspector-member-add">
          <Select value={newMemberId} onValueChange={setNewMemberId}>
            <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
              <SelectValue placeholder={bilingual('选择成员', 'Select member')} />
            </SelectTrigger>
            <SelectContent>
              {memberOptions.map((member) => (
                <SelectItem key={member.id} value={member.userId}>{member.displayName}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="collab-inspector-member-add-actions">
            <Select value={newMemberRole} onValueChange={(value) => setNewMemberRole(value as 'member' | 'manager')}>
              <SelectTrigger className="h-9 w-[120px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="member">{bilingual('成员', 'Member')}</SelectItem>
                <SelectItem value="manager">{bilingual('管理者', 'Manager')}</SelectItem>
              </SelectContent>
            </Select>
            <Button
              variant="outline"
              className="h-9 rounded-[12px] px-3 text-[12px]"
              onClick={() => void handleAddMember()}
              disabled={!newMemberId}
            >
              {bilingual('添加成员', 'Add member')}
            </Button>
          </div>
        </div>
        <div className="collab-inspector-helper">{bilingual('先选成员，再设角色并添加。成员标识默认截断显示，悬停可查看完整 ID。', 'Select a member first, then choose a role and add them. Member identifiers are truncated by default; hover to view the full ID.')}</div>
      </section>
      <div className="space-y-2.5">
        {members.map((member) => (
          <div key={member.user_id} className="collab-member-row">
            <div className="collab-member-identity">
              <div className="flex min-w-0 items-center gap-2">
                <div
                  className="collab-member-name"
                  title={teamMembers.find((item) => item.userId === member.user_id)?.displayName || member.user_id}
                >
                  {teamMembers.find((item) => item.userId === member.user_id)?.displayName || member.user_id}
                </div>
              </div>
              <div className="collab-member-id" title={member.user_id}>{formatCompactIdentifier(member.user_id)}</div>
            </div>
            <div className="collab-member-actions">
            <Select value={member.role} onValueChange={(value) => void handleUpdateMemberRole(member.user_id, value as 'owner' | 'manager' | 'member')}>
              <SelectTrigger className="h-8 w-[120px] rounded-[10px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="owner">{bilingual('所有者', 'Owner')}</SelectItem>
                <SelectItem value="manager">{bilingual('管理者', 'Manager')}</SelectItem>
                <SelectItem value="member">{bilingual('成员', 'Member')}</SelectItem>
              </SelectContent>
            </Select>
            <Button
              variant="ghost"
              size="sm"
              className="h-8 rounded-[10px] px-2.5 text-[12px]"
              onClick={() => void handleRemoveMember(member.user_id)}
            >
              {bilingual('移除', 'Remove')}
            </Button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );

  const renderSettingsContent = () => (
    <div className="collab-inspector-pane">
      <section className="collab-inspector-section">
        <div className="collab-inspector-section-head">
          <div className="min-w-0">
            <div className="collab-inspector-section-title">{bilingual('基本信息', 'Basic info')}</div>
            <div className="collab-inspector-section-meta">{bilingual('维护频道名称、说明、可见范围与默认 Agent。', 'Manage the channel name, description, visibility, and default agent.')}</div>
          </div>
        </div>
        <div className="space-y-3">
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">{bilingual('频道名称', 'Channel name')}</span>
            <Input
              value={form.name}
              onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
              placeholder={bilingual('频道名称', 'Channel name')}
              className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
            />
          </label>
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">{bilingual('频道说明', 'Channel description')}</span>
            <Textarea
              value={form.description}
              onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))}
              placeholder={bilingual('频道说明', 'Channel description')}
              className="min-h-[92px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
            />
          </label>
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">{bilingual('可见范围', 'Visibility')}</span>
            <Select value={form.visibility} onValueChange={(value) => setForm((prev) => ({ ...prev, visibility: value as ChatChannelVisibility }))}>
              <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
                <SelectValue />
              </SelectTrigger>
        <SelectContent>
          <SelectItem value="team_public">{bilingual('公开频道', 'Public channel')}</SelectItem>
          <SelectItem value="team_private">{bilingual('私密频道', 'Private channel')}</SelectItem>
        </SelectContent>
            </Select>
          </label>
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">{bilingual('默认 Agent', 'Default agent')}</span>
      <Select value={form.defaultAgentId} onValueChange={(value) => setForm((prev) => ({ ...prev, defaultAgentId: value }))}>
        <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
          <SelectValue placeholder={bilingual('默认 Agent', 'Default agent')} />
        </SelectTrigger>
        <SelectContent>
          {visibleAgents.map((agent) => (
            <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
          ))}
        </SelectContent>
      </Select>
          </label>
        </div>
      </section>
      <section className="collab-inspector-section">
        <div className="collab-inspector-section-head">
          <div className="min-w-0">
            <div className="collab-inspector-section-title">{bilingual('协作策略', 'Collaboration policy')}</div>
            <div className="collab-inspector-section-meta">{bilingual('设置管理 Agent 在这个频道里的主动度和推进方式。', 'Configure how proactively the managing agent participates and drives work in this channel.')}</div>
          </div>
        </div>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">{bilingual('Agent 主动度', 'Agent autonomy')}</span>
        <Select
          value={form.agentAutonomyMode}
          onValueChange={(value) =>
            setForm((prev) => ({ ...prev, agentAutonomyMode: value as ChatChannelAgentAutonomyMode }))
          }
        >
          <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="standard">{bilingual('标准模式', 'Standard mode')}</SelectItem>
            <SelectItem value="proactive">{bilingual('主动推进模式', 'Proactive mode')}</SelectItem>
            <SelectItem value="agent_lead">{bilingual('Agent 主导模式', 'Agent-led mode')}</SelectItem>
          </SelectContent>
        </Select>
        <div className="collab-inspector-helper">
          {getChannelAutonomyMeta(form.agentAutonomyMode).summary}
        </div>
        </label>
      </section>
      <section className="collab-inspector-section">
        <div className="collab-inspector-section-head">
          <div className="min-w-0">
            <div className="collab-inspector-section-title">{bilingual('频道记忆', 'Channel memory')}</div>
            <div className="collab-inspector-section-meta">{bilingual('管理 Agent 会长期参考这里的目标、参与人和产出物。', 'The managing agent will keep using these goals, participants, and outputs over time.')}</div>
          </div>
        </div>
        <div className="space-y-3">
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">{bilingual('频道目标', 'Channel goal')}</span>
          <Textarea
            value={form.channelGoal}
            onChange={(event) => setForm((prev) => ({ ...prev, channelGoal: event.target.value }))}
            placeholder={bilingual('这个频道主要围绕什么事情建立？', 'What is this channel mainly created for?')}
            className="min-h-[76px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">{bilingual('参与人', 'Participants')}</span>
          <Textarea
            value={form.participantNotes}
            onChange={(event) => setForm((prev) => ({ ...prev, participantNotes: event.target.value }))}
            placeholder={bilingual('主要谁会参与协作？谁是关键判断人？', 'Who mainly participates? Who makes the key decisions?')}
            className="min-h-[76px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">{bilingual('产出物', 'Outputs')}</span>
          <Textarea
            value={form.expectedOutputs}
            onChange={(event) => setForm((prev) => ({ ...prev, expectedOutputs: event.target.value }))}
            placeholder={bilingual('最终希望形成什么结果、文档、方案或交付物？', 'What result, document, plan, or deliverable do you want in the end?')}
            className="min-h-[76px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">{bilingual('协作方式', 'Collaboration style')}</span>
          <Input
            value={form.collaborationStyle}
            onChange={(event) => setForm((prev) => ({ ...prev, collaborationStyle: event.target.value }))}
            placeholder={bilingual('例如：偏讨论 / 偏方案 / 偏执行 / 偏评审 / 混合', 'For example: discussion-heavy / planning-heavy / execution-heavy / review-heavy / mixed')}
            className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        {channelDetail?.orchestrator_state?.last_heartbeat_at ? (
          <div className="collab-inspector-inline-note">
            {bilingual('最近心跳：', 'Latest heartbeat: ')}{formatDateTime(channelDetail.orchestrator_state.last_heartbeat_at)}
            {channelDetail.orchestrator_state.last_heartbeat_reason
              ? ` · ${channelDetail.orchestrator_state.last_heartbeat_reason}`
              : ''}
          </div>
        ) : null}
        </div>
      </section>
      <div className="flex justify-end">
        <Button className="h-9 rounded-[12px] px-3 text-[12px]" onClick={() => void handleSaveChannel()}>
          <Save className="mr-1 h-4 w-4" />{bilingual('保存频道设置', 'Save channel settings')}
        </Button>
      </div>
      <section className="collab-inspector-danger">
        <div className="collab-inspector-section-title">{bilingual('危险操作', 'Danger zone')}</div>
        <div className="collab-inspector-section-meta">
          {bilingual('可以选择只删除频道，保留所有文档；或者彻底删除频道和相关文档。', 'You can delete only the channel and keep all documents, or permanently delete the channel and related documents.')}
        </div>
        <div className="mt-3 flex justify-end">
          <Button variant="destructive" className="h-9 rounded-[12px] px-3 text-[12px]" onClick={() => setDeleteDialogOpen(true)}>
            {bilingual('删除频道', 'Delete channel')}
          </Button>
        </div>
      </section>
    </div>
  );

  const renderWorkspacePanelContent = () => {
    if (!channelDetail) return null;

    const currentThreadLabel = threadRootMessage
      ? currentThreadSummary
      : bilingual('当前还没有打开具体协作线程', 'No collaboration thread is currently open');
    const currentDelegationSummary = buildDelegationRuntimeSummary(
      threadDelegationRuntime,
    );

    return (
      <div className="collab-inspector-pane">
        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('项目空间', 'Project space')}</div>
                <span className="collab-inspector-count">coding</span>
              </div>
              <div className="collab-meta mt-1 text-[11px] leading-5">
                {bilingual('编程项目频道会把协作线程放到服务器工作区里执行。信息面板继续保留资料、AI 产出、成员和设置，这里只看代码现场。', 'Coding channels execute collaboration threads in a server workspace. The info panel still keeps docs, AI outputs, members, and settings; this view focuses only on the code context.')}
              </div>
            </div>
          </div>
          <div className="space-y-2">
            <div className="rounded-[14px] border border-border/70 bg-background/80 px-3 py-3">
              <div className="text-[11px] font-medium text-foreground">
                {channelDetail.workspace_display_name || channelDetail.name}
              </div>
              <div className="mt-1 text-[11px] text-muted-foreground">
                {channelTypeLabel(channelDetail.channel_type)}
              </div>
              <div className="mt-3 space-y-2 text-[11px] text-muted-foreground">
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('项目空间路径', 'Project space path')}</div>
                  <div className="mt-0.5 break-all font-mono">{channelDetail.workspace_path || bilingual('未绑定', 'Not bound')}</div>
                </div>
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('仓库路径', 'Repo path')}</div>
                  <div className="mt-0.5 break-all font-mono">{channelDetail.repo_path || bilingual('未生成', 'Not created')}</div>
                </div>
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('主检出路径', 'Main checkout path')}</div>
                  <div className="mt-0.5 break-all font-mono">
                    {channelDetail.main_checkout_path || channelDetail.repo_root || bilingual('未生成', 'Not created')}
                  </div>
                </div>
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('默认分支', 'Default branch')}</div>
                  <div className="mt-0.5">{channelDetail.repo_default_branch || 'main'}</div>
                </div>
              </div>
            </div>
          </div>
        </section>

        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('线程现场', 'Thread workspace')}</div>
              </div>
              <div className="collab-meta mt-1 text-[11px] leading-5">
                {bilingual('当前线程会在独立 worktree 中继续推进；没有打开线程时，这里只显示频道级项目空间。', 'The current thread continues in an isolated worktree. When no thread is open, this view shows only the channel-level project space.')}
              </div>
            </div>
          </div>
          <div className="rounded-[14px] border border-border/70 bg-background/80 px-3 py-3">
            <div className="text-[11px] font-medium text-foreground">{currentThreadLabel}</div>
            {threadRuntime ? (
              <div className="mt-3 space-y-2 text-[11px] text-muted-foreground">
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('线程现场路径', 'Thread workspace path')}</div>
                  <div className="mt-0.5 break-all font-mono">
                    {threadRuntime.thread_worktree_path || threadRuntime.workspace_path || bilingual('未绑定', 'Not bound')}
                  </div>
                </div>
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('线程分支', 'Thread branch')}</div>
                  <div className="mt-0.5">{threadRuntime.thread_branch || bilingual('未分配', 'Unassigned')}</div>
                </div>
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('仓库引用', 'Repo ref')}</div>
                  <div className="mt-0.5 break-all font-mono">{threadRuntime.thread_repo_ref || bilingual('未绑定', 'Not bound')}</div>
                </div>
                <div>
                  <div className="font-medium text-foreground/80">{bilingual('运行时会话', 'Runtime session')}</div>
                  <div className="mt-0.5 break-all font-mono">{threadRuntime.runtime_session_id || bilingual('未创建', 'Not created')}</div>
                </div>
              </div>
            ) : (
              <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                {bilingual('还没有打开具体线程，进入协作项后这里会显示对应的 worktree、分支和运行时会话。', 'No specific thread is open yet. After entering a collaboration item, this panel will show the worktree, branch, and runtime session.')}
              </div>
            )}
          </div>
        </section>

        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('协作运行态', 'Collaboration runtime')}</div>
                  <span
                    className={`rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                      threadDelegationRuntime?.status,
                    )}`}
                  >
                    {delegationRuntimeStatusLabel(threadDelegationRuntime?.status)}
                  </span>
              </div>
                <div className="collab-meta mt-1 text-[11px] leading-5">
                  {bilingual('查看当前线程是否发生了委托执行，以及每个 worker 的推进状态。', 'Inspect whether delegation happened in the current thread and how each worker is progressing.')}
                </div>
            </div>
          </div>
          <div className="rounded-[14px] border border-border/70 bg-background/80 px-3 py-3">
            <div className="text-[11px] leading-5 text-muted-foreground">
              {currentDelegationSummary}
            </div>
            {threadDelegationRuntime?.leader ? (
              <div className="mt-3 rounded-[12px] border border-border/60 bg-background px-3 py-2.5">
                <div className="flex items-center justify-between gap-2">
                  <div>
                      <div className="text-[11px] uppercase tracking-wide text-muted-foreground">{bilingual('协调者', 'Leader')}</div>
                      <div className="text-[13px] font-medium text-foreground">
                        {delegationLeaderTitle(threadDelegationRuntime.leader.title)}
                      </div>
                  </div>
                    <span
                      className={`rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                        threadDelegationRuntime.leader.status,
                      )}`}
                    >
                      {delegationRuntimeStatusLabel(threadDelegationRuntime.leader.status)}
                    </span>
                </div>
                {threadDelegationRuntime.leader.summary ? (
                  <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                    {threadDelegationRuntime.leader.summary}
                  </div>
                ) : null}
              </div>
            ) : null}
            <div className="mt-3 space-y-2">
              {threadDelegationRuntime?.workers?.length ? (
                threadDelegationRuntime.workers.map((worker) => (
                  <div
                    key={worker.worker_id}
                    className="rounded-[12px] border border-border/60 bg-background px-3 py-2.5"
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0">
                          <div className="text-[13px] font-medium text-foreground">
                            {delegationWorkerTitle(worker)}
                          </div>
                          <div className="mt-1 text-[11px] uppercase tracking-wide text-muted-foreground">
                            {delegationWorkerRoleLabel(worker.role)}
                          </div>
                        </div>
                        <span
                          className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                            worker.status,
                          )}`}
                        >
                          {delegationRuntimeStatusLabel(worker.status)}
                        </span>
                    </div>
                    {worker.summary ? (
                      <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                        {worker.summary}
                      </div>
                    ) : null}
                    {worker.result_summary ? (
                      <div className="mt-2 text-[11px] leading-5 text-foreground">
                        {worker.result_summary}
                      </div>
                    ) : null}
                    {worker.error ? (
                      <div className="mt-2 text-[11px] leading-5 text-status-error-text">
                        {worker.error}
                      </div>
                    ) : null}
                  </div>
                ))
              ) : (
                <div className="text-[11px] leading-5 text-muted-foreground">
                  {bilingual('当前线程还没有发生 subagent 或 swarm 运行。', 'No subagent or swarm execution has happened in this thread yet.')}
                </div>
              )}
            </div>
          </div>
        </section>

        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('项目结构', 'Project structure')}</div>
                <span className="collab-inspector-count">{workspaceCodeFiles.length}</span>
              </div>
              <div className="collab-meta mt-1 text-[11px] leading-5">
                {bilingual('只显示当前代码现场里的文件夹和代码文件名。打开线程时优先读取 thread worktree，没有线程时回退到主检出目录。', 'Only folders and code filenames from the current code context are shown. When a thread is open, it reads the thread worktree first; otherwise it falls back to the main checkout.')}
              </div>
            </div>
          </div>
          <div className="rounded-[14px] border border-border/70 bg-background/80 px-3 py-3">
            {workspaceCodeFilesRootPath ? (
              <div className="mb-3 text-[11px] text-muted-foreground">
                <div className="font-medium text-foreground/80">{bilingual('当前读取路径', 'Current path')}</div>
                <div className="mt-0.5 break-all font-mono">{workspaceCodeFilesRootPath}</div>
              </div>
            ) : null}
            {workspaceCodeFilesLoading ? (
              <div className="text-[11px] text-muted-foreground">{bilingual('正在读取代码文件…', 'Loading code files…')}</div>
            ) : workspaceCodeFilesError ? (
              <div className="text-[11px] text-destructive">{workspaceCodeFilesError}</div>
            ) : workspaceCodeFiles.length > 0 ? (
              <div className="space-y-3">
                <div className="max-h-[420px] space-y-1.5 overflow-y-auto pr-1">
                  {workspaceCodeTree}
                </div>
                {workspaceCodeFilesTruncated ? (
                  <div className="text-[11px] text-muted-foreground">
                    {bilingual('文件较多，当前只显示前 800 个代码文件。', 'There are many files, so only the first 800 code files are shown right now.')}
                  </div>
                ) : null}
              </div>
            ) : (
              <div className="text-[11px] leading-5 text-muted-foreground">
                {bilingual('当前代码现场里还没有识别到代码文件。', 'No code files were detected in the current code context yet.')}
              </div>
            )}
          </div>
        </section>

        <section className="collab-inspector-section">
          <div className="collab-inspector-section-head">
            <div className="min-w-0 flex-1">
              <div className="collab-inspector-section-header-row">
                <div className="collab-inspector-section-title">{bilingual('治理说明', 'Governance notes')}</div>
              </div>
            </div>
          </div>
          <div className="rounded-[14px] border border-border/70 bg-background/80 px-3 py-3 text-[11px] leading-5 text-muted-foreground">
            {bilingual('切回普通协作频道时，只会解绑频道和项目工作区的使用关系，不会删除底层 repo/worktree。后续工作区恢复、归档和删除治理，会继续落在这个独立工作区面板里。', 'Switching back to a general collaboration channel only detaches the channel from the project workspace. It does not delete the underlying repo or worktree. Later restore, archive, and delete governance stays in this dedicated workspace panel.')}
          </div>
        </section>
      </div>
    );
  };

  const renderDesktopInspector = () => {
    if (!channelDetail || isMobile) return null;
    const inspectorTab =
      sidePanelMode && sidePanelMode !== 'thread' && sidePanelMode !== 'workspace'
        ? sidePanelMode
        : null;
    if (!inspectorTab) return null;
    const inspectorTabs: Array<{ key: InspectorTabKey; label: string; count?: number; summary: string }> = [
      {
        key: 'documents',
        label: bilingual('资料', 'Docs'),
        count: channelDocuments.length,
        summary: bilingual('频道资料目录与已上传材料。只有附加到当前消息或线程的内容才会进入当前上下文。', 'Channel documents and uploaded material. Only content attached to the current message or thread enters the current context.'),
      },
      {
        key: 'ai_outputs',
        label: bilingual('AI 产出', 'AI outputs'),
        count: channelAiOutputs.length,
        summary: bilingual('Agent 在当前频道生成的草稿、总结和结果，可附加、归档或发布。', 'Drafts, summaries, and results generated by the agent in this channel. They can be attached, archived, or published.'),
      },
      {
        key: 'members',
        label: bilingual('成员', 'Members'),
        count: members.length,
        summary: bilingual('维护谁能看到这个频道、谁负责判断，以及各成员在频道中的角色。', 'Manage who can see the channel, who makes decisions, and what role each member has inside it.'),
      },
      {
        key: 'settings',
        label: bilingual('设置', 'Settings'),
        summary: bilingual('管理频道基本信息、Agent 主动度、频道记忆与删除策略。', 'Manage basic channel information, agent autonomy, channel memory, and deletion strategy.'),
      },
    ];
    const activeTab = inspectorTabs.find((item) => item.key === inspectorTab) || inspectorTabs[0];
    return (
      <div className="collab-inspector-shell w-[368px] shrink-0">
        <div className="flex h-full min-h-0 flex-col">
          <div className="collab-shell-header collab-inspector-header px-4 py-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex-1">
                <div className="collab-inspector-kicker">{bilingual('信息面板', 'Info panel')}</div>
                <div className="collab-inspector-header-row">
                  <div className="collab-inspector-title">{activeTab.label}</div>
                  {typeof activeTab.count === 'number' ? (
                    <span className="collab-inspector-count">{activeTab.count}</span>
                  ) : null}
                </div>
                <div className="collab-inspector-subtitle">{activeTab.summary}</div>
              </div>
              <button
                type="button"
                onClick={() => setSidePanelMode(null)}
                className="inline-flex h-8 w-8 items-center justify-center rounded-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="collab-inspector-tabs">
              {inspectorTabs.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  onClick={() => setSidePanelMode(item.key)}
                  className="collab-inspector-tab"
                  data-active={inspectorTab === item.key}
                >
                  <span>{item.label}</span>
                  {typeof item.count === 'number' ? (
                    <span className="collab-inspector-tab-badge">{item.count}</span>
                  ) : null}
                </button>
              ))}
            </div>
          </div>
          <div className="collab-inspector-body min-h-0 flex-1 overflow-y-auto px-4 py-4">
            {inspectorTab === 'documents' ? renderDocumentsInspector() : null}
            {inspectorTab === 'ai_outputs' ? renderAiOutputsInspector() : null}
            {inspectorTab === 'members' ? renderMembersContent() : null}
            {inspectorTab === 'settings' ? renderSettingsContent() : null}
          </div>
        </div>
      </div>
    );
  };

  const renderDesktopWorkspacePanel = () => {
    if (!channelDetail || isMobile || sidePanelMode !== 'workspace' || channelDetail.channel_type !== 'coding') {
      return null;
    }

    return (
      <div className="collab-inspector-shell w-[368px] shrink-0">
        <div className="flex h-full min-h-0 flex-col">
          <div className="collab-shell-header collab-inspector-header px-4 py-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex-1">
                <div className="collab-inspector-kicker">{bilingual('工作区面板', 'Workspace panel')}</div>
                <div className="collab-inspector-header-row">
                  <div className="collab-inspector-title">{bilingual('代码现场', 'Code context')}</div>
                </div>
                <div className="collab-inspector-subtitle">
                  {bilingual('查看频道绑定的项目空间、仓库主检出和当前线程现场。原来的信息面板保持不变，这里只承接编程相关上下文。', 'Inspect the channel-bound project space, main repo checkout, and current thread workspace. The original info panel stays unchanged; this panel handles only coding-related context.')}
                </div>
              </div>
              <button
                type="button"
                onClick={() => setSidePanelMode(null)}
                className="inline-flex h-8 w-8 items-center justify-center rounded-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </div>
          <div className="collab-inspector-body min-h-0 flex-1 overflow-y-auto px-4 py-4">
            {renderWorkspacePanelContent()}
          </div>
        </div>
      </div>
    );
  };

  const applyChannelPrefs = useCallback((channelId: string, prefs: ChatChannelUserPrefs) => {
    setChannels((prev) =>
      prev.map((channel) =>
        channel.channel_id === channelId
          ? {
              ...channel,
              pinned: prefs.pinned,
              muted: prefs.muted,
              last_visited_at: prefs.last_visited_at ?? channel.last_visited_at ?? null,
            }
          : channel,
      ),
    );
    setChannelDetail((prev) =>
      prev && prev.channel_id === channelId
        ? {
            ...prev,
            pinned: prefs.pinned,
            muted: prefs.muted,
            last_visited_at: prefs.last_visited_at ?? prev.last_visited_at ?? null,
          }
        : prev,
    );
  }, []);

  const handleTogglePinned = useCallback(
    async (channel: ChatChannelSummary) => {
      const nextPinned = !channel.pinned;
      applyChannelPrefs(channel.channel_id, {
        channel_id: channel.channel_id,
        team_id: channel.team_id,
        user_id: user?.id || '',
        pinned: nextPinned,
        muted: channel.muted,
        last_visited_at: channel.last_visited_at ?? null,
      });
      try {
        const prefs = await chatApi.updateChannelPrefs(channel.channel_id, {
          pinned: nextPinned,
        });
        applyChannelPrefs(channel.channel_id, prefs);
      } catch (prefError) {
        console.error('Failed to update channel pinned state:', prefError);
        applyChannelPrefs(channel.channel_id, {
          channel_id: channel.channel_id,
          team_id: channel.team_id,
          user_id: user?.id || '',
          pinned: channel.pinned,
          muted: channel.muted,
          last_visited_at: channel.last_visited_at ?? null,
        });
        setError(bilingual('更新频道置顶状态失败，请稍后再试。', 'Failed to update the pinned state. Please try again later.'));
      }
    },
    [applyChannelPrefs, user?.id],
  );

  const handleToggleMuted = useCallback(
    async (channel: ChatChannelSummary) => {
      const nextMuted = !channel.muted;
      applyChannelPrefs(channel.channel_id, {
        channel_id: channel.channel_id,
        team_id: channel.team_id,
        user_id: user?.id || '',
        pinned: channel.pinned,
        muted: nextMuted,
        last_visited_at: channel.last_visited_at ?? null,
      });
      try {
        const prefs = await chatApi.updateChannelPrefs(channel.channel_id, {
          muted: nextMuted,
        });
        applyChannelPrefs(channel.channel_id, prefs);
      } catch (prefError) {
        console.error('Failed to update channel mute state:', prefError);
        applyChannelPrefs(channel.channel_id, {
          channel_id: channel.channel_id,
          team_id: channel.team_id,
          user_id: user?.id || '',
          pinned: channel.pinned,
          muted: channel.muted,
          last_visited_at: channel.last_visited_at ?? null,
        });
        setError(bilingual('更新频道静音状态失败，请稍后再试。', 'Failed to update the muted state. Please try again later.'));
      }
    },
    [applyChannelPrefs, user?.id],
  );

  const sortedChannels = useMemo(() => {
    const items = [...channels];
    items.sort((a, b) => {
      const aHasUnread = a.unread_count > 0;
      const bHasUnread = b.unread_count > 0;
      return Number(b.pinned) - Number(a.pinned)
        || Number(a.muted) - Number(b.muted)
        || Number(bHasUnread) - Number(aHasUnread)
        || String(channelLastActivity(b)).localeCompare(String(channelLastActivity(a)))
        || a.name.localeCompare(b.name, 'zh-CN');
    });
    return items;
  }, [channels]);

  const filteredChannels = useMemo(() => {
    const keyword = channelSearch.trim().toLowerCase();
    if (!keyword) {
      return sortedChannels;
    }
    return sortedChannels.filter((channel) => {
      const haystacks = [
        channel.name,
        channel.description || '',
        channel.last_message_preview || '',
      ];
      return haystacks.some((value) => value.toLowerCase().includes(keyword));
    });
  }, [channelSearch, sortedChannels]);

  const collaborationItems = useMemo(() => {
    return collaborationRootMessages
      .map((message) => {
      const rootId = message.thread_root_id || message.message_id;
      const preview = (message.summary_text || pairedAssistantByRootId.get(rootId)?.content_text || '').trim();
      const titleSource = message.content_text?.trim() || preview || bilingual('未命名协作项', 'Untitled collaboration item');
      const firstLine = titleSource.split('\n')[0] || titleSource;
      const title = firstLine.length > 28 ? `${firstLine.slice(0, 28)}…` : firstLine;
      const fallbackAgentNames =
        (message.recent_agent_names && message.recent_agent_names.length > 0)
          ? normalizeAgentDisplayNames(message.recent_agent_names)
          : message.agent_id
            ? visibleAgents
                .filter((agent) => agent.id === message.agent_id)
                .map((agent) => normalizeAgentDisplayName(agent.name))
                .filter(Boolean)
            : [];
      return {
        message,
        rootId,
        title,
        preview: preview.length > 48 ? `${preview.slice(0, 48)}…` : preview,
        documentCount: threadDocumentCountMap.get(rootId) || 0,
        aiOutputCount: threadAiOutputCountMap.get(rootId) || 0,
        hasAi: message.has_ai_participation,
        recentAgents: fallbackAgentNames,
      };
    });
  }, [collaborationRootMessages, pairedAssistantByRootId, threadAiOutputCountMap, threadDocumentCountMap, visibleAgents]);

  const collaborationSurfaceCounts = useMemo(() => {
    const counts: Record<CollaborationSurfaceFilter, number> = {
      all: collaborationItems.length,
      temporary: 0,
      issue: 0,
    };
    collaborationItems.forEach((item) => {
      if (item.message.surface === 'issue') {
        counts.issue += 1;
        return;
      }
      counts.temporary += 1;
    });
    return counts;
  }, [collaborationItems]);

  const collaborationItemsBySurface = useMemo(() => {
    if (workSurfaceFilter === 'all') {
      return collaborationItems;
    }
    return collaborationItems.filter((item) => item.message.surface === workSurfaceFilter);
  }, [collaborationItems, workSurfaceFilter]);

  const collaborationStatusCounts = useMemo(() => {
    const counts: Record<CollaborationStatusFilter, number> = {
      all: collaborationItemsBySurface.length,
      proposed: 0,
      active: 0,
      awaiting_confirmation: 0,
      adopted: 0,
      rejected: 0,
    };
    collaborationItemsBySurface.forEach((item) => {
      const status = (item.message.display_status || 'active') as Exclude<CollaborationStatusFilter, 'all'>;
      counts[status] += 1;
    });
    return counts;
  }, [collaborationItemsBySurface]);

  const currentWorkOrUpdateItems = useMemo(() => {
    if (workStatusFilter === 'all') {
      return collaborationItemsBySurface;
    }
    return collaborationItemsBySurface.filter(
      (item) => (item.message.display_status || 'active') === workStatusFilter,
    );
  }, [collaborationItemsBySurface, workStatusFilter]);

  const discussionAreaMessages = useMemo(
    () =>
      mainChannelMessages.filter(
        (message) =>
          message.display_kind !== 'collaboration'
          && !(message.display_kind === 'suggestion' && message.display_status === 'rejected'),
      ),
    [mainChannelMessages],
  );
  const discussionTimelineEntries = useMemo(() => {
    const entries: Array<
      | { kind: 'day'; key: string; label: string }
      | { kind: 'message'; key: string; message: ChannelRenderMessage; groupedWithPrevious: boolean }
    > = [];
    let previousMessage: ChannelRenderMessage | null = null;
    let previousDayKey: string | null = null;
    discussionAreaMessages.forEach((message) => {
      const dayKey = formatDiscussionDay(message.created_at);
      if (dayKey && dayKey !== previousDayKey) {
        entries.push({ kind: 'day', key: `day-${dayKey}`, label: dayKey });
        previousDayKey = dayKey;
        previousMessage = null;
      }
      const groupedWithPrevious = isGroupedDiscussionMessage(previousMessage, message);
      entries.push({
        kind: 'message',
        key: message.message_id,
        message,
        groupedWithPrevious,
      });
      previousMessage = message;
    });
    return entries;
  }, [discussionAreaMessages]);

  const desktopThreadOpen = Boolean(
    !isMobile && desktopThreadMode && threadRootId && threadRootMessage,
  );

  const inspectorTab = (
    sidePanelMode && sidePanelMode !== 'thread' && sidePanelMode !== 'workspace'
      ? sidePanelMode
      : null
  ) as InspectorTabKey | null;
  const isCodingChannel = channelDetail?.channel_type === 'coding';
  const openWorkspacePreview = useCallback((filePath: string) => {
    if (!channelDetail?.channel_id) return;
    const url = chatApi.getChannelWorkspacePreviewUrl(
      channelDetail.channel_id,
      filePath,
      threadRootId,
    );
    window.open(url, '_blank', 'noopener,noreferrer');
  }, [channelDetail?.channel_id, threadRootId]);
  const workspaceCodeTree = useMemo(() => {
    type TreeNode = { name: string; children: Map<string, TreeNode>; files: string[] };
    const root: TreeNode = { name: '', children: new Map(), files: [] };
    for (const file of workspaceCodeFiles) {
      const parts = file.split('/').filter(Boolean);
      if (parts.length === 0) continue;
      let current = root;
      for (const segment of parts.slice(0, -1)) {
        let next = current.children.get(segment);
        if (!next) {
          next = { name: segment, children: new Map(), files: [] };
          current.children.set(segment, next);
        }
        current = next;
      }
      current.files.push(parts[parts.length - 1]);
    }
    const renderNode = (node: TreeNode, pathPrefix = ''): ReactNode => {
      const folders = Array.from(node.children.values()).sort((a, b) => a.name.localeCompare(b.name));
      const files = [...node.files].sort((a, b) => a.localeCompare(b));
      return (
        <>
          {folders.map((folder) => {
            const nextPath = pathPrefix ? `${pathPrefix}/${folder.name}` : folder.name;
            return (
              <div key={`dir:${nextPath}`} className="space-y-1">
                <div className="rounded-[10px] bg-muted/[0.05] px-2.5 py-1.5 text-[11px] font-medium text-foreground">
                  {folder.name}/
                </div>
                <div className="ml-3 space-y-1 border-l border-border/50 pl-2">
                  {renderNode(folder, nextPath)}
                </div>
              </div>
            );
          })}
          {files.map((file) => {
            const filePath = pathPrefix ? `${pathPrefix}/${file}` : file;
            const canPreview = workspaceFileSupportsBrowserPreview(filePath);
            return (
              <div
                key={`file:${filePath}`}
                className="rounded-[10px] border border-border/60 bg-background px-2.5 py-1.5 text-[11px] text-foreground"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="break-all font-mono">{file}</span>
                  {canPreview ? (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="h-6 shrink-0 rounded-[8px] px-2 text-[10px]"
                      onClick={() => openWorkspacePreview(filePath)}
                    >
                      {bilingual('打开预览', 'Open preview')}
                    </Button>
                  ) : null}
                </div>
              </div>
            );
          })}
        </>
      );
    };
    return renderNode(root);
  }, [openWorkspacePreview, workspaceCodeFiles]);

  useEffect(() => {
    if (sidePanelMode === 'workspace' && !isCodingChannel) {
      setSidePanelMode(null);
    }
  }, [isCodingChannel, sidePanelMode]);

  useEffect(() => {
    if (!selectedChannelId || !isCodingChannel) {
      setWorkspaceCodeFiles([]);
      setWorkspaceCodeFilesRootPath(null);
      setWorkspaceCodeFilesTruncated(false);
      setWorkspaceCodeFilesLoading(false);
      setWorkspaceCodeFilesError(null);
    }
  }, [isCodingChannel, selectedChannelId]);

  useEffect(() => {
    if (sidePanelMode !== 'workspace' || !selectedChannelId || !isCodingChannel) {
      return;
    }
    let cancelled = false;
    setWorkspaceCodeFilesLoading(true);
    setWorkspaceCodeFilesError(null);
    void chatApi
      .getChannelWorkspaceFiles(selectedChannelId, threadRootId)
      .then((payload) => {
        if (cancelled) return;
        setWorkspaceCodeFiles(payload.code_files || []);
        setWorkspaceCodeFilesRootPath(payload.root_path || null);
        setWorkspaceCodeFilesTruncated(Boolean(payload.truncated));
      })
      .catch((loadError) => {
        if (cancelled) return;
        console.error('Failed to load workspace code files:', loadError);
        setWorkspaceCodeFiles([]);
        setWorkspaceCodeFilesRootPath(null);
        setWorkspaceCodeFilesTruncated(false);
        setWorkspaceCodeFilesError(bilingual('加载代码文件失败，请稍后再试。', 'Failed to load code files. Please try again later.'));
      })
      .finally(() => {
        if (cancelled) return;
        setWorkspaceCodeFilesLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [isCodingChannel, selectedChannelId, sidePanelMode, threadRootId]);

  const currentAutonomyMeta = useMemo(
    () => getChannelAutonomyMeta(channelDetail?.orchestrator_state?.agent_autonomy_mode || 'standard'),
    [channelDetail?.orchestrator_state?.agent_autonomy_mode],
  );

  const desktopWorkspaceMode = surfaceView === 'update'
    ? 'discussion'
    : desktopThreadOpen
      ? 'work'
      : 'worklist';

  const handleSelectDesktopWorkspaceMode = useCallback((nextMode: 'discussion' | 'work' | 'worklist') => {
    if (nextMode === 'discussion') {
      setSurfaceView('update');
      return;
    }
    setSurfaceView('work');
    if (nextMode === 'worklist') {
      setDesktopThreadMode(false);
      return;
    }
    if (threadRootId && threadRootMessage) {
      setWorkSurfaceFilter(threadRootMessage.surface === 'issue' ? 'issue' : 'temporary');
      setDesktopThreadMode(true);
      return;
    }
    if (selectedChannelId && collaborationItems.length > 0) {
      const firstItem = collaborationItems[0].message;
      setWorkSurfaceFilter(firstItem.surface === 'issue' ? 'issue' : 'temporary');
      setDesktopThreadMode(true);
      void loadThread(selectedChannelId, firstItem.message_id);
      return;
    }
    setDesktopThreadMode(true);
  }, [collaborationItems, loadThread, selectedChannelId, threadRootId, threadRootMessage]);

  const renderDesktopRootComposer = (mode: 'discussion' | 'work') => {
    const composerDisabled = sending || (!composeText.trim() && selectedCapabilityRefs.length === 0);
    const isDiscussionMode = mode === 'discussion';

    return (
      <div className="collab-composer-shell mt-1.5 px-4 py-1.5">
        <input
          ref={fileInputRef}
          type="file"
          accept={FILE_ACCEPT}
          multiple
          className="hidden"
          onChange={(event) => void handleUploadToChannelFolder(event.target.files)}
        />
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <span className="collab-kicker">{isDiscussionMode ? bilingual('讨论模式', 'Discussion mode') : bilingual('新建协作项', 'New collaboration item')}</span>
              <span className="collab-meta">
                {isDiscussionMode
                  ? (rootStartsTemporaryCollaboration
                      ? bilingual(`已检测到 @${rootMentionedAgentLabel}，发送后会打开临时协作会话`, `@${rootMentionedAgentLabel} detected. Sending will open a temporary collaboration session.`)
                      : bilingual('先商量，再决定是否进入协作', 'Discuss first, then decide whether to enter collaboration'))
                  : bilingual('这里只发送明确要推进的具体工作', 'Only send concrete work that should be actively driven here')}
              </span>
            </div>
          </div>
        </div>

        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          <div className="relative min-w-[220px] flex-1 md:max-w-[280px] md:flex-none">
            {renderComposerTargetTrigger(
              'root',
              'h-8 rounded-[10px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none',
            )}
          </div>
          <Button
            type="button"
            variant="outline"
            className="h-8 rounded-[10px] px-2.5 text-[11px]"
            onClick={() => {
              setCapabilityDetailKey(null);
              setShowCapabilityPicker(true);
            }}
          >
            {bilingual('技能', 'Skills')}
          </Button>
          <Button
            type="button"
            variant="outline"
            className="h-8 rounded-[10px] px-2.5 text-[11px]"
            onClick={() => setShowDocPicker(true)}
          >
            {bilingual('附件', 'Attachments')}
          </Button>
          <Button
            type="button"
            variant="outline"
            className="h-8 rounded-[10px] px-2.5 text-[11px]"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploadingDocument}
          >
            {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传', 'Upload')}
          </Button>
        </div>

        {(attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            {attachedDocs.map((doc) => (
              <span key={doc.id} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-1 text-[11px] text-muted-foreground">
                <Paperclip className="h-3 w-3 shrink-0" />
                <span className="max-w-[180px] truncate">{doc.display_name || doc.name}</span>
                <button
                  type="button"
                  onClick={() => {
                    setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                    setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                  }}
                  className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                  title={bilingual('移除文档', 'Remove document')}
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
            {selectedCapabilities.map((item) => (
              <span key={item.key} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.1] px-2.5 py-1 text-[11px] text-primary">
                <Sparkles className="h-3 w-3 shrink-0" />
                <span className="max-w-[180px] truncate">{item.name}</span>
                <button
                  type="button"
                  onClick={() => removeCapabilityRef(item.ref)}
                  className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
                  title={bilingual('移除能力', 'Remove capability')}
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            ))}
          </div>
        ) : null}

        <div className="mt-1 rounded-[12px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-1.5 py-1">
          <Textarea
            ref={rootComposerTextareaRef}
            value={composeText}
            onChange={handleRootComposerChange}
            onSelect={handleRootComposerSelect}
            onKeyDown={(event) => handleComposerMentionKeyDown('root', event)}
            rows={1}
            className="!min-h-0 max-h-[220px] resize-none border-0 bg-transparent px-1 py-0.5 text-[12px] leading-5 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
            placeholder={
              isDiscussionMode
                ? (rootStartsTemporaryCollaboration
                    ? bilingual(`和 @${rootMentionedAgentLabel} 先聊清楚问题、背景和下一步…`, `Talk with @${rootMentionedAgentLabel} first to clarify the problem, context, and next step…`)
                    : bilingual('把问题、判断、同步或 @成员发到讨论区…', 'Send discussion content here to sync, decide, or @mention teammates…'))
                : bilingual('描述这条要推进的具体工作、预期结果和下一步…', 'Describe the work to drive, the expected result, and the next step…')
            }
          />
          <div className="mt-0.5 flex flex-wrap items-center justify-end gap-1.5 px-1 pt-0.5">
          {isDiscussionMode ? (
            <>
                <Button
                  onClick={() => void handleSend(undefined, rootStartsTemporaryCollaboration ? undefined : 'discussion')}
                  disabled={composerDisabled}
                  className="h-8 rounded-[10px] px-3 text-[11px]"
                >
                  <Send className="mr-1 h-3.5 w-3.5" />
                  {rootStartsTemporaryCollaboration ? bilingual('开始临时协作', 'Start temporary collaboration') : bilingual('发送讨论', 'Send discussion')}
                </Button>
              </>
            ) : (
              <>
                <Button
                  variant={rootStartsTemporaryCollaboration ? 'default' : 'outline'}
                  onClick={() => void handleSend(undefined, rootStartsTemporaryCollaboration ? undefined : 'discussion')}
                  disabled={composerDisabled}
                  className="h-8 rounded-[10px] px-3 text-[11px]"
                >
                  <Send className="mr-1 h-3.5 w-3.5" />
                    {rootStartsTemporaryCollaboration ? bilingual('开始临时协作', 'Start temporary collaboration') : bilingual('发到讨论区', 'Send to discussion')}
                </Button>
                {!rootStartsTemporaryCollaboration ? (
                  <Button
                    onClick={() => void handleSend(undefined, 'work')}
                    disabled={composerDisabled}
                    className="h-8 rounded-[10px] px-3 text-[11px]"
                  >
                    <Sparkles className="mr-1 h-3.5 w-3.5" />
                    {bilingual('创建协作项', 'Create collaboration item')}
                  </Button>
                ) : null}
              </>
            )}
          </div>
        </div>
        {error ? <div className="pt-2 text-[12px] text-destructive">{error}</div> : null}
      </div>
    );
  };

  const renderDesktopThreadComposer = () => (
    <div className="collab-composer-shell mt-2 px-4 py-1.5">
      <input
        ref={fileInputRef}
        type="file"
        accept={FILE_ACCEPT}
        multiple
        className="hidden"
        onChange={(event) => void handleUploadToChannelFolder(event.target.files)}
      />
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="collab-kicker">{bilingual('协作模式', 'Collaboration mode')}</span>
          <span className="collab-meta">{bilingual('围绕当前协作项继续补充、修正和推进', 'Continue adding context, corrections, and progress around the current collaboration item')}</span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-[180px]">
            {renderComposerTargetTrigger(
              'thread',
              'h-8 rounded-[10px] border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none',
            )}
          </div>
          <Button type="button" variant="outline" className="h-8 rounded-[10px] px-2.5 text-[11px]" onClick={() => { setCapabilityDetailKey(null); setShowCapabilityPicker(true); }}>
            {bilingual('技能', 'Skills')}
          </Button>
          <Button type="button" variant="outline" className="h-8 rounded-[10px] px-2.5 text-[11px]" onClick={() => setShowDocPicker(true)}>
            {bilingual('附件', 'Attachments')}
          </Button>
          <Button type="button" variant="outline" className="h-8 rounded-[10px] px-2.5 text-[11px]" onClick={() => fileInputRef.current?.click()} disabled={uploadingDocument}>
            {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传', 'Upload')}
          </Button>
        </div>
      </div>

      {(attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
        <div className="mt-1 flex flex-wrap items-center gap-1.5">
          {attachedDocs.map((doc) => (
            <span key={doc.id} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-1 text-[11px] text-muted-foreground">
              <Paperclip className="h-3 w-3 shrink-0" />
              <span className="max-w-[160px] truncate">{doc.display_name || doc.name}</span>
              <button
                type="button"
                onClick={() => {
                  setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                  setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                }}
                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
              >
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
          {selectedCapabilities.map((item) => (
            <span key={item.key} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.1] px-2.5 py-1 text-[11px] text-primary">
              <Sparkles className="h-3 w-3 shrink-0" />
              <span className="max-w-[160px] truncate">{item.name}</span>
              <button
                type="button"
                onClick={() => removeCapabilityRef(item.ref)}
                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
              >
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      ) : null}

        <div className="mt-1 rounded-[12px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-1.5 py-1">
          <Textarea
            ref={threadComposerTextareaRef}
            value={threadComposeText}
            onChange={handleThreadComposerChange}
            onSelect={handleThreadComposerSelect}
            onKeyDown={(event) => handleComposerMentionKeyDown('thread', event)}
            rows={1}
            className="!min-h-0 max-h-[220px] resize-none border-0 bg-transparent px-1 py-0.5 text-[12px] leading-5 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
            placeholder={bilingual('围绕这条协作线程继续对话、补充判断或澄清下一步…', 'Continue this collaboration thread, add judgment, or clarify the next step…')}
          />
          <div className="mt-0.5 flex items-center justify-end gap-2 px-1 pt-0.5">
            <Button
              onClick={() => void handleSend(threadRootId)}
              disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
              className="h-8 rounded-[10px] px-3 text-[11px]"
            >
              <MessageSquareReply className="mr-1 h-3.5 w-3.5" />
              {bilingual('继续推进', 'Continue')}
            </Button>
        </div>
      </div>
      {error ? <div className="pt-2 text-[12px] text-destructive">{error}</div> : null}
    </div>
  );

  const renderAutonomyGuide = () => (
    <div className="collab-autonomy-guide">
      <div className="collab-autonomy-guide-title">{bilingual('协作方式说明', 'Collaboration guide')}</div>
      <div className="collab-autonomy-guide-list">
        {CHANNEL_AUTONOMY_OPTIONS.map((item) => (
          <div
            key={item.mode}
            className="collab-autonomy-guide-item"
            data-active={currentAutonomyMeta.mode === item.mode}
          >
            <div className="collab-autonomy-guide-label-row">
              <span className="collab-autonomy-guide-label">{item.label}</span>
              {currentAutonomyMeta.mode === item.mode ? (
                <span className="collab-autonomy-guide-current">{bilingual('当前频道', 'Current channel')}</span>
              ) : null}
            </div>
            <div className="collab-autonomy-guide-summary">{item.summary}</div>
          </div>
        ))}
      </div>
    </div>
  );

  const renderDesktopWorkspace = () => {
    if (!channelDetail) return null;
    const activeThread = desktopThreadOpen ? threadRootMessage : null;

    return (
      <div className="flex min-h-0 flex-1 flex-col">
          <div className="collab-stage-shell flex min-h-0 flex-1 flex-col">
            <div className="collab-shell-header px-4 pb-1 pt-2">
            <div className="flex flex-wrap items-center gap-2">
              <div className="min-w-0 flex flex-1 items-center gap-2">
                <span className="collab-kicker shrink-0">{bilingual('智能协作频道', 'AI collaboration channel')}</span>
                <h1 className="collab-display-title min-w-0 truncate">
                  {channelDetail.name}
                </h1>
                {channelDetail.is_processing ? (
                  <span className="collab-toolbar-pill collab-micro shrink-0" data-active="true">
                    {bilingual('处理中', 'Processing')}
                  </span>
                ) : null}
              </div>
              <Button
                variant={inspectorTab ? 'default' : 'outline'}
                size="sm"
                className="h-8 shrink-0 rounded-[10px] px-2.5 text-[11px]"
                onClick={() =>
                  setSidePanelMode((current) =>
                    current && current !== 'thread' && current !== 'workspace' ? null : 'documents',
                  )
                }
              >
                {bilingual('信息面板', 'Info panel')}
              </Button>
              {isCodingChannel ? (
                <Button
                  variant={sidePanelMode === 'workspace' ? 'default' : 'outline'}
                  size="sm"
                  className="h-8 shrink-0 rounded-[10px] px-2.5 text-[11px]"
                  onClick={() => setSidePanelMode((current) => (current === 'workspace' ? null : 'workspace'))}
                >
                  {bilingual('工作区', 'Workspace')}
                </Button>
              ) : null}
            </div>

            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              <span className="collab-toolbar-pill collab-micro">
                {channelVisibilityLabel(channelDetail.visibility)}
              </span>
              <span className="collab-toolbar-pill collab-micro">
                {normalizeAgentDisplayName(channelDetail.default_agent_name)}
              </span>
              <span className="collab-toolbar-pill collab-micro">
                {bilingual(`${channelDetail.member_count} 位成员`, `${channelDetail.member_count} members`)}
              </span>
              <span
                className="collab-autonomy-pill"
                data-mode={currentAutonomyMeta.mode}
              >
                {currentAutonomyMeta.shortLabel}
              </span>
              <button
                type="button"
                onClick={() => setAutonomyGuideOpen((prev) => !prev)}
                className="collab-autonomy-trigger"
                aria-expanded={autonomyGuideOpen}
              >
                {bilingual('协作方式说明', 'Collaboration guide')}
              </button>
              <span className="collab-meta truncate">
                {channelDetail.description || bilingual('讨论模式负责团队商讨，协作模式负责单条工作的推进，协作列表负责统一查看全部协作项。', 'Discussion mode supports team planning, collaboration mode advances a single work item, and the worklist gives a unified view of all collaboration items.')}
              </span>
            </div>
            {autonomyGuideOpen ? renderAutonomyGuide() : null}

            <div className="collab-mode-switch mt-1">
              {([
                { key: 'discussion', label: bilingual('讨论模式', 'Discussion mode'), hint: bilingual('团队群聊', 'Team chat'), count: discussionAreaMessages.length },
                {
                  key: 'work',
                  label: bilingual('协作模式', 'Collaboration mode'),
                  hint: activeThread ? collaborationSurfaceLabel(activeThread.surface) : bilingual('当前工作', 'Current work'),
                  count: activeThread ? 1 : 0,
                },
                {
                  key: 'worklist',
                  label: bilingual('协作列表', 'Collaboration list'),
                  hint: workSurfaceFilter === 'all' ? bilingual('临时 + 正式', 'Temporary + formal') : collaborationSurfaceFilterLabel(workSurfaceFilter),
                  count: collaborationSurfaceCounts[workSurfaceFilter],
                },
              ] as Array<{ key: 'discussion' | 'work' | 'worklist'; label: string; hint: string; count: number }>).map((item) => {
                const isActive = desktopWorkspaceMode === item.key;
                return (
                  <button
                    key={item.key}
                    type="button"
                    onClick={() => handleSelectDesktopWorkspaceMode(item.key)}
                    className="collab-mode-tab"
                    data-active={isActive}
                  >
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[12px] font-medium text-foreground">{item.label}</span>
                        <span className="text-[10px] text-muted-foreground">{item.hint}</span>
                      </div>
                    </div>
                    <span className="collab-mode-tab-badge">
                      {item.count}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="flex min-h-0 flex-1 flex-col px-5 pb-2 pt-0">
            {desktopWorkspaceMode === 'discussion' ? (
              <>
                <section className="collab-column-shell flex min-h-0 flex-1 flex-col">
                  <div className="collab-column-header px-4 py-0">
                    <div className="collab-header-rail">
                      <div className="collab-header-rail-main">
                        <span className="collab-kicker">{bilingual('讨论模式', 'Discussion mode')}</span>
                        <span className="collab-header-title">{bilingual('团队群聊与商讨窗口', 'Team chat and planning space')}</span>
                        <span className="collab-header-description">{bilingual('用于商量方向、同步结果、提醒成员，并决定哪些事情进入协作。', 'Use this space to align on direction, sync results, remind members, and decide which work should enter collaboration.')}</span>
                      </div>
                      <span className="collab-toolbar-pill collab-micro collab-header-count">{bilingual(`${discussionAreaMessages.length} 条讨论`, `${discussionAreaMessages.length} discussions`)}</span>
                    </div>
                  </div>
                  <div
                    ref={messageScrollRef}
                    className="collab-column-scroll flex flex-col px-4 pb-2 pt-0"
                    onScroll={() => {
                      stickMainToBottomRef.current = isNearBottom(messageScrollRef.current);
                    }}
                  >
                    {loadingMessages ? (
                      <div className="flex h-full items-center justify-center py-10">
                        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                      </div>
                    ) : discussionAreaMessages.length === 0 ? (
                      <div className="collab-empty-shell flex h-full min-h-[320px] items-center justify-center px-8 text-center">
                        <div>
                          <div className="collab-section-title">{bilingual('讨论区还没有内容', 'The discussion area is empty')}</div>
                          <div className="collab-meta mt-3 max-w-[460px] leading-6">
                            {bilingual('先在这里发起团队讨论，确认方向、@相关成员，等事情清楚之后再转进协作模式。', 'Start team discussion here first, confirm direction, @mention relevant members, and move into collaboration mode after the work is clear.')}
                          </div>
                        </div>
                      </div>
                    ) : (
                      <div className="flex flex-1 flex-col justify-end gap-4">
                        {discussionTimelineEntries.map((entry) =>
                          entry.kind === 'day'
                            ? <DiscussionDayDivider key={entry.key} label={entry.label} />
                            : renderMessage(entry.message, entry.groupedWithPrevious),
                        )}
                        <div ref={messageEndRef} />
                      </div>
                    )}
                  </div>
                </section>
                {renderDesktopRootComposer('discussion')}
              </>
            ) : desktopWorkspaceMode === 'work' ? (
              <>
                <section className="collab-column-shell flex min-h-0 flex-1 flex-col">
                  <div className="collab-column-header px-4 py-0">
                    <div className="collab-thread-rail">
                      <div className="collab-thread-rail-main">
                        <span className="collab-kicker shrink-0">{bilingual('协作模式', 'Collaboration mode')}</span>
                        <span className="collab-thread-title">
                          {activeThread ? currentThreadSummary : bilingual('进入一条具体协作项继续推进', 'Enter a specific collaboration item to continue')}
                        </span>
                        {activeThread ? (
                          <span className={`collab-status-chip ${collaborationSurfaceTone(activeThread.surface)}`}>
                            {collaborationSurfaceLabel(activeThread.surface)}
                          </span>
                        ) : null}
                        {activeThread ? (
                          <span className={`collab-status-chip ${workStatusTone(activeThread.display_status, activeThread.surface)}`}>
                            {workStatusLabel(activeThread.display_status, activeThread.surface)}
                          </span>
                        ) : null}
                        {activeThread ? (
                          <ThreadStatusBar replyCount={currentThreadReplyCount} documentCount={currentThreadDocumentCount} aiOutputCount={currentThreadAiOutputCount} compact />
                        ) : null}
                          {activeThread && threadRuntime ? (
                            <div className="mt-2 rounded-[12px] border border-[hsl(var(--ui-line-soft))/0.6] bg-[hsl(var(--ui-surface-panel-strong))/0.22] px-3 py-2 text-[11px] text-muted-foreground">
                              {bilingual('线程现场：', 'Thread workspace: ')}{threadRuntime.thread_worktree_path || threadRuntime.workspace_path || bilingual('未绑定', 'Not bound')}
                              {threadRuntime.thread_branch ? ` · ${bilingual('分支：', 'Branch: ')}${threadRuntime.thread_branch}` : ''}
                            </div>
                          ) : null}
                          <span className="collab-thread-description">
                          {activeThread
                            ? activeThread.surface === 'temporary'
                              ? bilingual('先围绕问题澄清、追问和试探方向，再决定是否升级成正式协作。', 'Clarify the problem, ask follow-up questions, and explore direction first before deciding whether to promote it into formal collaboration.')
                              : bilingual('围绕一件正式协作项持续推进、补充判断；长时间静默后会自动同步阶段进展到讨论区。', 'Continue advancing a formal collaboration item and add judgment as needed. After long silence, stage progress syncs back to discussion automatically.')
                            : bilingual('协作模式一次只处理一条具体工作。', 'Collaboration mode handles one concrete work item at a time.')}
                        </span>
                        {activeThread ? (
                          <button
                            type="button"
                            onClick={() => setThreadRootExpanded((prev) => !prev)}
                            className="collab-thread-origin-link"
                          >
                            <ChevronRight className={`h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform ${threadRootExpanded ? 'rotate-90' : ''}`} />
                            <span className="collab-thread-origin-label">{bilingual('协作起点', 'Origin')}</span>
                            <span className="collab-thread-origin-text">{activeThread.author_name} · {bilingual('点击查看最初发起内容', 'Click to view the original start message')}</span>
                          </button>
                        ) : null}
                      </div>
                    </div>

                    {activeThread ? (
                      <div className="collab-thread-action-row">
                        {!['adopted', 'rejected'].includes(activeThread.display_status || '') ? (
                          <>
                            <Button type="button" size="sm" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestAdoptThreadConfirm(activeThread)}>{bilingual('标记采用', 'Mark as adopted')}</Button>
                            {activeThread.surface === 'temporary' ? (
                              <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestMarkThreadHighlightedConfirm(activeThread)}>{bilingual('升级为正式协作', 'Promote to formal collaboration')}</Button>
                            ) : null}
                            {activeThread.display_status !== 'awaiting_confirmation' ? (
                              <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestAwaitingConfirmationConfirm(activeThread)}>{bilingual('等判断', 'Needs decision')}</Button>
                            ) : null}
                            <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestSyncThreadResultConfirm(activeThread)}>{bilingual('同步到讨论', 'Sync to discussion')}</Button>
                            <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestArchiveThreadConfirm(activeThread)}>{bilingual('未采用', 'Rejected')}</Button>
                          </>
                        ) : null}
                      </div>
                    ) : null}
                    {activeThread && threadRootExpanded ? <ThreadRootCard message={activeThread} /> : null}
                  </div>
                  <div
                    ref={activeThread ? threadScrollRef : undefined}
                    className="collab-column-scroll flex flex-col px-4 pb-2 pt-0"
                    onScroll={activeThread ? () => {
                      stickThreadToBottomRef.current = isNearBottom(threadScrollRef.current);
                    } : undefined}
                  >
                    {activeThread ? (
                      <div className="flex flex-1 flex-col justify-end gap-4">
                        {threadMessages.map((message) => (
                          <ThreadFlowMessage
                            key={message.message_id}
                            message={message}
                            isOwn={message.author_user_id === user?.id}
                            showFullDiagnostics={canViewFullDiagnostics}
                          />
                        ))}
                        <div ref={threadEndRef} />
                      </div>
                    ) : (
                      <div className="collab-empty-shell flex h-full min-h-[320px] items-center justify-center px-8 text-center">
                        <div>
                          <div className="collab-section-title">{bilingual('还没有打开具体协作项', 'No collaboration item is open yet')}</div>
                          <div className="collab-meta mt-3 max-w-[460px] leading-6">{bilingual('协作模式一次只处理一条具体工作。直接用上方三态切换到协作列表，或者打开最近的协作项继续推进。', 'Collaboration mode handles one concrete item at a time. Use the state switch above to open the worklist, or open the latest collaboration item to continue.')}</div>
                          <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
                            {collaborationItems.length > 0 ? (
                              <Button type="button" onClick={() => void handleOpenThread(collaborationItems[0].message)}>
                                {bilingual('打开最近协作项', 'Open latest collaboration item')}
                              </Button>
                            ) : null}
                          </div>
                        </div>
                      </div>
                    )}
                  </div>
                </section>
                {activeThread ? renderDesktopThreadComposer() : renderDesktopRootComposer('work')}
              </>
            ) : (
              <>
                <section className="collab-column-shell flex min-h-0 flex-1 flex-col">
                  <div className="collab-column-header px-4 py-0">
                    <div className="space-y-1.5">
                      <div className="collab-worklist-header">
                        <div className="collab-header-rail-main">
                          <span className="collab-kicker">{bilingual('协作列表', 'Collaboration list')}</span>
                          <span className="collab-header-title">
                            {workSurfaceFilter === 'temporary'
                              ? bilingual('先聊明白的临时协作', 'Temporary collaboration for clarifying first')
                              : workSurfaceFilter === 'issue'
                                ? bilingual('正在推进的正式协作', 'Formal collaboration in progress')
                                : bilingual('查看全部协作项，再决定进入哪条工作', 'Review all collaboration items, then choose which work to enter')}
                          </span>
                          <span className="collab-header-description">
                            {workSurfaceFilter === 'temporary'
                              ? bilingual('这里集中查看 @Agent 打开的探讨线程和待澄清事项。', 'This area gathers exploratory threads opened with @Agent and items that still need clarification.')
                              : workSurfaceFilter === 'issue'
                                ? bilingual('这里集中查看已经进入正式推进的工作；静默一段时间后，系统会把阶段进展同步到讨论区。', 'This area gathers work already in formal execution. After a quiet period, stage updates sync back to discussion.')
                                : bilingual('先按协作类型分区，再按状态筛选，选中后进入协作模式处理单条工作。', 'Split items by collaboration type first, then filter by status and enter collaboration mode for a single item.')}
                          </span>
                        </div>
                        <span className="collab-toolbar-pill collab-micro collab-header-count">
                          {bilingual(`${currentWorkOrUpdateItems.length} 条`, `${currentWorkOrUpdateItems.length} items`)}
                        </span>
                      </div>
                      <div className="space-y-1.5">
                        <div className="collab-worklist-filter-strip">
                          {COLLABORATION_SURFACE_FILTER_OPTIONS.map((item) => (
                            <button
                              key={item.key}
                              type="button"
                              className="collab-worklist-filter"
                              data-active={workSurfaceFilter === item.key}
                              data-tone={item.tone}
                              onClick={() => setWorkSurfaceFilter(item.key)}
                            >
                              <span>{item.label}</span>
                              <span className="collab-worklist-filter-badge">{collaborationSurfaceCounts[item.key]}</span>
                            </button>
                          ))}
                        </div>
                        <div className="collab-worklist-filter-strip">
                          {COLLABORATION_STATUS_FILTER_OPTIONS.map((item) => (
                            <button
                              key={item.key}
                              type="button"
                              className="collab-worklist-filter"
                              data-active={workStatusFilter === item.key}
                              data-tone={item.tone}
                              onClick={() => setWorkStatusFilter(item.key)}
                            >
                              <span>{item.label}</span>
                              <span className="collab-worklist-filter-badge">{collaborationStatusCounts[item.key]}</span>
                            </button>
                          ))}
                        </div>
                      </div>
                    </div>
                  </div>
                  <div className="collab-column-scroll px-4 pb-2 pt-0">
                    {currentWorkOrUpdateItems.length === 0 ? (
                      <div className="collab-empty-shell flex h-full min-h-[320px] items-center justify-center px-8 text-center">
                        <div>
                          <div className="collab-section-title">
                            {collaborationWorklistTitle(workSurfaceFilter, workStatusFilter)}
                          </div>
                          <div className="collab-meta mt-3 max-w-[460px] leading-6">
                            {collaborationWorklistDescription(workSurfaceFilter, workStatusFilter)}
                          </div>
                        </div>
                      </div>
                    ) : (
                      <div className="space-y-2">
                        {currentWorkOrUpdateItems.map((item) => {
                          const selected = threadRootId === item.message.message_id;
                          return (
                            <button key={item.message.message_id} type="button" className="collab-work-card w-full px-3.5 py-3 text-left" data-selected={selected} onClick={() => void handleOpenThread(item.message)}>
                              <div className="flex items-start justify-between gap-3">
                                <div className="min-w-0 flex-1">
                                <div className="flex flex-wrap items-center gap-2">
                                  <span className={`rounded-full px-2 py-0.5 text-[10px] ${collaborationSurfaceTone(item.message.surface)}`}>
                                    {collaborationSurfaceLabel(item.message.surface)}
                                  </span>
                                  <span className={`rounded-full px-2 py-0.5 text-[10px] ${workStatusTone(item.message.display_status, item.message.surface)}`}>
                                    {workStatusLabel(item.message.display_status, item.message.surface)}
                                  </span>
                                  <span className="rounded-full border border-border/60 px-2 py-0.5 text-[10px] text-muted-foreground">
                                    {item.message.source_kind === 'agent' ? bilingual('Agent 发起', 'Started by agent') : bilingual('成员发起', 'Started by member')}
                                    </span>
                                  </div>
                                  <div className="mt-2 truncate text-[13px] font-semibold text-foreground">{item.title}</div>
                                  <div className="mt-1 line-clamp-2 text-[11px] leading-5 text-muted-foreground">{item.preview || item.message.content_text}</div>
                                  {collaborationSurfaceHint(item.message.surface) ? (
                                    <div className="mt-1 collab-surface-hint">
                                      {collaborationSurfaceHint(item.message.surface)}
                                    </div>
                                  ) : null}
                                  <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px]">
                                    <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{bilingual(`${item.message.reply_count} 条回复`, `${item.message.reply_count} replies`)}</span>
                                    {item.recentAgents.length > 0 ? (
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{item.recentAgents.slice(0, 2).join(' · ')}</span>
                                    ) : null}
                                    {item.documentCount > 0 ? (
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{bilingual(`${item.documentCount} 资料`, `${item.documentCount} docs`)}</span>
                                    ) : null}
                                    {item.aiOutputCount > 0 ? (
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{bilingual(`${item.aiOutputCount} 个 AI 产出`, `${item.aiOutputCount} AI outputs`)}</span>
                                    ) : null}
                                  </div>
                                </div>
                                <div className="shrink-0 pt-1 text-muted-foreground">
                                  <ChevronRight className="h-4 w-4" />
                                </div>
                              </div>
                            </button>
                          );
                        })}
                      </div>
                    )}
                  </div>
                </section>
                {renderDesktopRootComposer('work')}
              </>
            )}
          </div>
        </div>
      </div>
    );
  };

  const renderDesktopWorkspaceLegacy = () => {
    if (!channelDetail) return null;
    const activeThread = desktopThreadOpen ? threadRootMessage : null;

    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="collab-stage-shell flex min-h-0 flex-1 flex-col">
          <div className="collab-shell-header px-6 py-4">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div className="min-w-0 flex-1">
                <div className="collab-kicker">{bilingual('智能协作频道', 'AI collaboration channel')}</div>
                <div className="mt-2 flex flex-wrap items-center gap-2">
                  <h1 className="collab-display-title min-w-0 truncate">
                    {channelDetail.name}
                  </h1>
                  <span className="collab-toolbar-pill collab-micro">
                    {channelVisibilityLabel(channelDetail.visibility)}
                  </span>
                  <span className="collab-toolbar-pill collab-micro">
                    {normalizeAgentDisplayName(channelDetail.default_agent_name)}
                  </span>
                  <span className="collab-toolbar-pill collab-micro">
                    {bilingual(`${channelDetail.member_count} 位成员`, `${channelDetail.member_count} members`)}
                  </span>
                  {channelDetail.is_processing ? (
                    <span className="collab-toolbar-pill collab-micro" data-active="true">
                      {bilingual('正在处理中', 'Processing')}
                    </span>
                  ) : null}
                </div>
                <p className="collab-meta mt-3 max-w-4xl leading-6">
                  {channelDetail.description || bilingual('讨论区负责商量、提醒、澄清与同步；协作项负责真正进入执行推进。频道资料和 AI 产出只在需要时展开查看。', 'Discussion handles planning, reminders, clarification, and sync. Collaboration items handle real execution. Channel docs and AI outputs expand only when needed.')}
                </p>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <Button
                  variant={inspectorTab ? 'default' : 'outline'}
                  size="sm"
                  className="h-9 rounded-[999px] px-4 text-[11px]"
                  onClick={() =>
                    setSidePanelMode((current) =>
                      current && current !== 'thread' ? null : 'documents',
                    )
                  }
                >
                  {bilingual('信息面板', 'Info panel')}
                </Button>
              </div>
            </div>
          </div>

          <div className="flex min-h-0 flex-1 flex-col px-5 pb-5 pt-5">
            <div className="grid min-h-0 flex-1 grid-cols-[minmax(0,1.12fr)_minmax(360px,0.74fr)] gap-4">
              <section className="collab-column-shell collab-timeline-shell">
                <div className="collab-column-header px-5 py-4">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="collab-kicker">{bilingual('讨论区', 'Discussion')}</div>
                      <div className="collab-section-title mt-2">{bilingual('重点讨论与商量', 'Main discussion and planning')}</div>
                      <div className="collab-meta mt-2 max-w-[560px] leading-5">
                        {bilingual('频道里的主要对话窗口。这里承接成员讨论、Agent 建议、阶段总结和结果同步，不直接替代协作执行。', 'This is the main conversation area in the channel. It carries member discussion, agent suggestions, stage summaries, and result sync, but does not replace collaboration execution.')}
                      </div>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="collab-toolbar-pill collab-micro">
                        {bilingual(`${discussionAreaMessages.length} 条消息`, `${discussionAreaMessages.length} messages`)}
                      </span>
                    </div>
                  </div>
                </div>

                <div
                  ref={messageScrollRef}
                  className="collab-column-scroll px-5 py-5"
                  onScroll={() => {
                    stickMainToBottomRef.current = isNearBottom(messageScrollRef.current);
                  }}
                >
                  {loadingMessages ? (
                    <div className="flex h-full items-center justify-center py-10">
                      <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                    </div>
                  ) : discussionAreaMessages.length === 0 ? (
                    <div className="collab-empty-shell flex h-full min-h-[280px] items-center justify-center px-8 text-center">
                      <div>
                        <div className="collab-section-title">{bilingual('讨论区还没有内容', 'The discussion area is empty')}</div>
                        <div className="collab-meta mt-3 max-w-[420px] leading-6">
                          {bilingual('这里用于公开同步、提醒、@成员，以及由管理 Agent 发出的建议卡、总结卡和结果卡。', 'Use this area for public sync, reminders, @mentions, and suggestion/summary/result cards generated by the managing agent.')}
                        </div>
                      </div>
                    </div>
                  ) : (
                    <div className="space-y-4">
                      {discussionTimelineEntries.map((entry) =>
                        entry.kind === 'day'
                          ? <DiscussionDayDivider key={entry.key} label={entry.label} />
                          : renderMessage(entry.message, entry.groupedWithPrevious),
                      )}
                    </div>
                  )}
                  <div ref={messageEndRef} />
                </div>
              </section>

              <section className="collab-column-shell">
                <div className="collab-column-header px-5 py-4">
                  {activeThread ? (
                    <div className="space-y-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div className="min-w-0 flex flex-1 items-center gap-2">
                          <button
                            type="button"
                            onClick={closeThreadPanel}
                            className="collab-toolbar-pill collab-micro"
                          >
                            <ArrowLeft className="h-3.5 w-3.5" />
                            {bilingual('返回协作项', 'Back to collaboration')}
                          </button>
                          <div className="min-w-0 flex-1">
                            <div className="collab-kicker">{bilingual('协作项', 'Collaboration item')}</div>
                            <div className="collab-section-title mt-2 truncate">
                              {currentThreadSummary}
                            </div>
                          </div>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className={`rounded-full px-2.5 py-1 collab-micro ${collaborationSurfaceTone(activeThread.surface)}`}>
                            {collaborationSurfaceLabel(activeThread.surface)}
                          </span>
                          <span className={`rounded-full px-2.5 py-1 collab-micro ${workStatusTone(activeThread.display_status, activeThread.surface)}`}>
                            {workStatusLabel(activeThread.display_status, activeThread.surface)}
                          </span>
                        </div>
                      </div>

                      <ThreadStatusBar
                        replyCount={currentThreadReplyCount}
                        documentCount={currentThreadDocumentCount}
                        aiOutputCount={currentThreadAiOutputCount}
                        compact
                      />

                      {!['adopted', 'rejected'].includes(activeThread.display_status || '') ? (
                        <div className="flex flex-wrap items-center gap-2">
                          <Button
                            type="button"
                            size="sm"
                            className="h-8 rounded-[999px] px-3 text-[11px]"
                            onClick={() => requestAdoptThreadConfirm(activeThread)}
                          >
                            {bilingual('标记采用', 'Mark as adopted')}
                          </Button>
                          {activeThread.surface === 'temporary' ? (
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              className="h-8 rounded-[999px] px-3 text-[11px]"
                              onClick={() => requestMarkThreadHighlightedConfirm(activeThread)}
                            >
                              {bilingual('升级为正式协作', 'Promote to formal collaboration')}
                            </Button>
                          ) : null}
                          {activeThread.display_status !== 'awaiting_confirmation' ? (
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              className="h-8 rounded-[999px] px-3 text-[11px]"
                              onClick={() => requestAwaitingConfirmationConfirm(activeThread)}
                            >
                              {bilingual('等你判断', 'Needs your decision')}
                            </Button>
                          ) : null}
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-8 rounded-[999px] px-3 text-[11px]"
                            onClick={() => requestSyncThreadResultConfirm(activeThread)}
                          >
                            {bilingual('同步结果', 'Sync result')}
                          </Button>
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-8 rounded-[999px] px-3 text-[11px]"
                            onClick={() => requestArchiveThreadConfirm(activeThread)}
                          >
                            {bilingual('标记未采用', 'Mark as rejected')}
                          </Button>
                        </div>
                      ) : null}

                      <button
                        type="button"
                        onClick={() => setThreadRootExpanded((prev) => !prev)}
                        className="flex w-full items-center gap-2 rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.6] bg-[hsl(var(--ui-surface-panel-strong))/0.76] px-3.5 py-3 text-left transition-colors hover:border-[hsl(var(--ui-line-strong))/0.7]"
                      >
                        <ChevronRight className={`h-4 w-4 shrink-0 text-muted-foreground transition-transform ${threadRootExpanded ? 'rotate-90' : ''}`} />
                        <div className="min-w-0 flex-1">
                          <div className="collab-kicker">{bilingual('协作起点', 'Origin')}</div>
                          <div className="collab-meta mt-1 truncate">
                            {activeThread.author_name} · {bilingual('点击查看这条协作项最初的发起内容', 'Click to view the original starting message')}
                          </div>
                        </div>
                      </button>
                      {threadRootExpanded ? <ThreadRootCard message={activeThread} /> : null}
                    </div>
                  ) : (
                    <div className="space-y-3">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="collab-kicker">{bilingual('协作项', 'Collaboration item')}</div>
                          <div className="collab-section-title mt-2">
                            {workSurfaceFilter === 'temporary'
                              ? bilingual('先聊明白的临时协作', 'Temporary collaboration for clarification first')
                              : workSurfaceFilter === 'issue'
                                ? bilingual('正在推进的正式协作', 'Formal collaboration in progress')
                                : bilingual('进入协作执行', 'Enter collaboration execution')}
                          </div>
                          <div className="collab-meta mt-2 max-w-[320px] leading-5">
                            {workSurfaceFilter === 'temporary'
                              ? bilingual('这里集中查看 @Agent 打开的探讨线程、补充上下文和待澄清事项。', 'This view gathers exploratory threads opened with @Agent, supporting context, and items awaiting clarification.')
                              : workSurfaceFilter === 'issue'
                                ? bilingual('这里集中查看已经进入正式推进的工作；静默 1 小时后，阶段进展会同步到讨论区。', 'This view gathers work already in formal execution. After one hour of silence, stage progress syncs back to discussion.')
                                : bilingual('先按协作类型分区，再按状态筛选，选中后进入协作模式处理单条工作。', 'Split work by collaboration type first, then filter by status and enter collaboration mode for a single item.')}
                          </div>
                        </div>
                        <span className="collab-toolbar-pill collab-micro">
                          {bilingual(`${currentWorkOrUpdateItems.length} 条`, `${currentWorkOrUpdateItems.length} items`)}
                        </span>
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        {COLLABORATION_SURFACE_FILTER_OPTIONS.map((item) => (
                          <button
                            key={item.key}
                            type="button"
                            className="collab-toolbar-pill collab-micro"
                            data-active={workSurfaceFilter === item.key}
                            onClick={() => setWorkSurfaceFilter(item.key)}
                          >
                            <span>{item.label}</span>
                            <span className="rounded-full bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-1.5 py-0.5 text-[10px] text-inherit">
                              {collaborationSurfaceCounts[item.key]}
                            </span>
                          </button>
                        ))}
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        {COLLABORATION_STATUS_FILTER_OPTIONS.map((item) => (
                          <button
                            key={item.key}
                            type="button"
                            className="collab-toolbar-pill collab-micro"
                            data-active={workStatusFilter === item.key}
                            onClick={() => setWorkStatusFilter(item.key)}
                          >
                            <span>{item.label}</span>
                            <span className="rounded-full bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-1.5 py-0.5 text-[10px] text-inherit">
                              {collaborationStatusCounts[item.key]}
                            </span>
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                </div>

                <div
                  ref={desktopThreadOpen ? threadScrollRef : undefined}
                  className="collab-column-scroll px-5 py-5"
                  onScroll={
                    desktopThreadOpen
                      ? () => {
                          stickThreadToBottomRef.current = isNearBottom(threadScrollRef.current);
                        }
                      : undefined
                  }
                >
                  {activeThread ? (
                    <div className="space-y-4">
                      {threadMessages.map((message) => (
                        <ThreadFlowMessage
                          key={message.message_id}
                          message={message}
                          isOwn={message.author_user_id === user?.id}
                          showFullDiagnostics={canViewFullDiagnostics}
                        />
                      ))}
                      <div ref={threadEndRef} />
                    </div>
                  ) : currentWorkOrUpdateItems.length === 0 ? (
                    <div className="collab-empty-shell flex h-full min-h-[280px] items-center justify-center px-8 text-center">
                      <div>
                        <div className="collab-section-title">{bilingual('还没有协作项', 'No collaboration items yet')}</div>
                        <div className="collab-meta mt-3 max-w-[420px] leading-6">
                          {bilingual('先在讨论区把事情商量清楚，再把真正要推进的事项送进协作项。', 'Clarify the work in discussion first, then move the real execution items into collaboration.')}
                        </div>
                      </div>
                    </div>
                  ) : (
                    <div className="space-y-3">
                      {currentWorkOrUpdateItems.map((item) => {
                        const selected = threadRootId === item.message.message_id;
                        return (
                          <button
                            key={item.message.message_id}
                            type="button"
                            className="collab-work-card w-full p-4 text-left"
                            data-selected={selected}
                            onClick={() => void handleOpenThread(item.message)}
                          >
                            <div className="flex items-start justify-between gap-3">
                              <div className="min-w-0 flex-1">
                                <div className="flex flex-wrap items-center gap-2">
                                  <span className={`rounded-full px-2.5 py-1 collab-micro ${collaborationSurfaceTone(item.message.surface)}`}>
                                    {collaborationSurfaceLabel(item.message.surface)}
                                  </span>
                                  <span className={`rounded-full px-2.5 py-1 collab-micro ${workStatusTone(item.message.display_status, item.message.surface)}`}>
                                    {workStatusLabel(item.message.display_status, item.message.surface)}
                                  </span>
                                  <span className="ml-auto collab-meta">
                                    {formatDateTime(item.message.created_at)}
                                  </span>
                                </div>
                                <div className="mt-3 truncate text-[13px] font-semibold text-foreground">
                                  {item.title}
                                </div>
                                <div className="collab-meta mt-2 line-clamp-3 leading-5">
                                  {item.preview || item.message.content_text}
                                </div>
                                {collaborationSurfaceHint(item.message.surface) ? (
                                  <div className="mt-2 collab-surface-hint">
                                    {collaborationSurfaceHint(item.message.surface)}
                                  </div>
                                ) : null}
                                <div className="mt-3 flex flex-wrap items-center gap-2">
                                  <span className="collab-toolbar-pill collab-micro">
                                    {bilingual(`${item.message.reply_count} 条回复`, `${item.message.reply_count} replies`)}
                                  </span>
                                  {item.documentCount > 0 ? (
                                    <span className="collab-toolbar-pill collab-micro">
                                      {bilingual(`${item.documentCount} 资料`, `${item.documentCount} docs`)}
                                    </span>
                                  ) : null}
                                  {item.aiOutputCount > 0 ? (
                                    <span className="collab-toolbar-pill collab-micro">
                                      {bilingual(`${item.aiOutputCount} 个 AI 产出`, `${item.aiOutputCount} AI outputs`)}
                                    </span>
                                  ) : null}
                                </div>
                              </div>
                              <ChevronRight className="mt-1 h-4 w-4 shrink-0 text-muted-foreground" />
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              </section>
            </div>

            {!activeThread ? (
              <div className="collab-composer-shell mt-4 px-5 py-4">
                <input
                  ref={fileInputRef}
                  type="file"
                  accept={FILE_ACCEPT}
                  multiple
                  className="hidden"
                  onChange={(event) => void handleUploadToChannelFolder(event.target.files)}
                />
                <div className="flex flex-wrap items-start justify-between gap-4">
                  <div className="min-w-0">
                    <div className="collab-kicker">{bilingual('消息输入区', 'Message composer')}</div>
                    <div className="collab-section-title mt-2">{bilingual('讨论先行，必要时进入协作', 'Discuss first, enter collaboration only when needed')}</div>
                    <div className="collab-meta mt-2 max-w-[520px] leading-5">
                      {bilingual('默认先发到讨论区；只有当你明确要推进执行时，再直接创建协作项。', 'By default messages go to discussion first. Only create a collaboration item directly when you clearly want to drive execution.')}
                    </div>
                  </div>
                </div>

                <div className="mt-4 flex flex-wrap items-center gap-2">
                  <div className="relative min-w-[220px] flex-1 md:max-w-[280px] md:flex-none">
                    {renderComposerTargetTrigger(
                      'root',
                      'h-9 rounded-[999px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.86] px-3 text-[11px] shadow-none',
                    )}
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    className="h-9 rounded-[999px] px-3 text-[11px]"
                    onClick={() => {
                      setCapabilityDetailKey(null);
                      setShowCapabilityPicker(true);
                    }}
                  >
                    {bilingual('技能', 'Skills')}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    className="h-9 rounded-[999px] px-3 text-[11px]"
                    onClick={() => setShowDocPicker(true)}
                  >
                    {bilingual('附件', 'Attachments')}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    className="h-9 rounded-[999px] px-3 text-[11px]"
                    onClick={() => fileInputRef.current?.click()}
                    disabled={uploadingDocument}
                  >
                    {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传', 'Upload')}
                  </Button>
                </div>

                {(attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                  <div className="mt-4 flex flex-wrap items-center gap-2">
                    {attachedDocs.map((doc) => (
                      <span key={doc.id} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-1 text-[11px] text-muted-foreground">
                        <Paperclip className="h-3 w-3 shrink-0" />
                        <span className="max-w-[180px] truncate">{doc.display_name || doc.name}</span>
                        <button
                          type="button"
                          onClick={() => {
                            setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                            setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                          }}
                          className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                          title={bilingual('移除文档', 'Remove document')}
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </span>
                    ))}
                    {selectedCapabilities.map((item) => (
                      <span key={item.key} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.1] px-2.5 py-1 text-[11px] text-primary">
                        <Sparkles className="h-3 w-3 shrink-0" />
                        <span className="max-w-[180px] truncate">{item.name}</span>
                        <button
                          type="button"
                          onClick={() => removeCapabilityRef(item.ref)}
                          className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
                          title={bilingual('移除能力', 'Remove capability')}
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </span>
                    ))}
                  </div>
                ) : null}

                <div className="relative mt-4 rounded-[24px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.92] p-2.5">
                  <div className="px-2 pb-1 collab-meta">
                    {rootStartsTemporaryCollaboration
                    ? `A temporary collaboration session will open around @${rootMentionedAgentLabel} first for discussion and clarification, without treating it as a formal collaboration item yet.`
                        : bilingual('将发送到讨论区，适合同步、提醒、确认和 @成员；如果你要立即推进执行，可以直接创建协作项。', 'This will be sent to discussion, which is best for syncing, reminding, confirming, and @mentioning members. If you want to drive execution immediately, create a collaboration item directly.')}
                  </div>
                  <Textarea
                    ref={rootComposerTextareaRef}
                    value={composeText}
                    onChange={handleRootComposerChange}
                    onSelect={handleRootComposerSelect}
                    onKeyDown={(event) => handleComposerMentionKeyDown('root', event)}
                    className="min-h-[132px] resize-none border-0 bg-transparent px-2 py-2 pr-36 text-[13px] leading-6 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
                    placeholder={
                      rootStartsTemporaryCollaboration
                    ? `Talk with @${rootMentionedAgentLabel} first to clarify the problem, context, and next step…`
                        : bilingual('发送讨论内容，和团队同步、提醒、澄清或 @成员…', 'Send a discussion message to sync, remind, clarify, or @mention team members…')
                    }
                  />
                  <div className="absolute bottom-3 right-3 flex items-center gap-2">
                    {!rootStartsTemporaryCollaboration ? (
                      <Button
                        variant="outline"
                        onClick={() => void handleSend(undefined, 'work')}
                        disabled={sending || (!composeText.trim() && selectedCapabilityRefs.length === 0)}
                        className="h-10 rounded-[999px] px-4 text-[12px]"
                      >
                        <Sparkles className="mr-1 h-3.5 w-3.5" />
                        {bilingual('开始协作项', 'Create collaboration item')}
                      </Button>
                    ) : null}
                    <Button
                      onClick={() => void handleSend(undefined, rootStartsTemporaryCollaboration ? undefined : 'discussion')}
                      disabled={sending || (!composeText.trim() && selectedCapabilityRefs.length === 0)}
                      className="h-10 rounded-[999px] px-4 text-[12px]"
                    >
                      <Send className="mr-1 h-3.5 w-3.5" />
                      {rootStartsTemporaryCollaboration ? bilingual('开始临时协作', 'Start temporary collaboration') : bilingual('发送讨论', 'Send discussion')}
                    </Button>
                  </div>
                </div>
                {error ? <div className="pt-3 text-[12px] text-destructive">{error}</div> : null}
              </div>
            ) : (
              <div className="collab-composer-shell mt-4 px-5 py-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="collab-kicker">{bilingual('线程回复', 'Thread reply')}</span>
                    <span className="collab-meta">{bilingual('围绕当前协作项继续补充、修正和推进', 'Continue adding context, corrections, and progress around the current collaboration item')}</span>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="relative min-w-[180px]">
                      {renderComposerTargetTrigger(
                        'thread',
                        'h-8 rounded-[999px] border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--ui-surface-panel-strong))/0.86] px-3 text-[11px] shadow-none',
                      )}
                    </div>
                    <Button type="button" variant="outline" className="h-8 rounded-[999px] px-3 text-[11px]" onClick={() => { setCapabilityDetailKey(null); setShowCapabilityPicker(true); }}>
                      {bilingual('技能', 'Skills')}
                    </Button>
                    <Button type="button" variant="outline" className="h-8 rounded-[999px] px-3 text-[11px]" onClick={() => setShowDocPicker(true)}>
                      {bilingual('附件', 'Attachments')}
                    </Button>
                    <Button type="button" variant="outline" className="h-8 rounded-[999px] px-3 text-[11px]" onClick={() => fileInputRef.current?.click()} disabled={uploadingDocument}>
                      {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传', 'Upload')}
                    </Button>
                  </div>
                </div>

                {(attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                  <div className="mt-3 flex flex-wrap items-center gap-2">
                    {attachedDocs.map((doc) => (
                      <span key={doc.id} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--ui-surface-panel-strong))/0.82] px-2.5 py-1 text-[11px] text-muted-foreground">
                        <Paperclip className="h-3 w-3 shrink-0" />
                        <span className="max-w-[160px] truncate">{doc.display_name || doc.name}</span>
                        <button
                          type="button"
                          onClick={() => {
                            setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                            setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                          }}
                          className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </span>
                    ))}
                    {selectedCapabilities.map((item) => (
                      <span key={item.key} className="inline-flex max-w-full items-center gap-1 rounded-full border border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.1] px-2.5 py-1 text-[11px] text-primary">
                        <Sparkles className="h-3 w-3 shrink-0" />
                        <span className="max-w-[160px] truncate">{item.name}</span>
                        <button
                          type="button"
                          onClick={() => removeCapabilityRef(item.ref)}
                          className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </span>
                    ))}
                  </div>
                ) : null}

                <div className="relative mt-3 rounded-[22px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.9] p-2">
                  <Textarea
                    ref={threadComposerTextareaRef}
                    value={threadComposeText}
                    onChange={handleThreadComposerChange}
                    onSelect={handleThreadComposerSelect}
                    onKeyDown={(event) => handleComposerMentionKeyDown('thread', event)}
                    className="min-h-[110px] resize-none border-0 bg-transparent px-2 py-2 pr-32 text-[13px] leading-6 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
                    placeholder={bilingual('围绕这条协作项继续推进、补充判断或提出下一步…', 'Continue this collaboration item, add judgment, or propose the next step…')}
                  />
                  <div className="absolute bottom-3 right-3">
                    <Button
                      onClick={() => void handleSend(threadRootId)}
                      disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
                      className="h-9 rounded-[999px] px-4 text-[12px]"
                    >
                      <MessageSquareReply className="mr-1 h-3.5 w-3.5" />
                      {bilingual('回复协作项', 'Reply to collaboration')}
                    </Button>
                  </div>
                </div>
                {error ? <div className="pt-3 text-[12px] text-destructive">{error}</div> : null}
              </div>
            )}
          </div>
        </div>
      </div>
    );
  };

  void renderDesktopWorkspaceLegacy;

  return (
    <div className="collab-workspace flex h-full min-h-0 min-w-0 flex-1 overflow-hidden">
      <div className="collab-sidebar-shell flex w-full shrink-0 flex-col md:w-[296px] xl:w-[308px]">
        <div className="collab-shell-header px-4 py-4">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="collab-kicker">{bilingual('频道列表', 'Channel list')}</div>
              <div className="collab-section-title mt-2">{bilingual('智能协作频道', 'AI collaboration channels')}</div>
              <div className="collab-meta mt-2 leading-5">{bilingual('从这里进入具体频道，在讨论区商量，在协作项里推进执行。', 'Enter specific channels from here, discuss in the discussion area, and drive execution inside collaboration items.')}</div>
            </div>
            <Button
              size="icon"
              variant="outline"
              className="h-10 w-10 shrink-0 rounded-full border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.86] shadow-none"
              onClick={() => setCreateOpen(true)}
              title={bilingual('新建频道', 'Create channel')}
            >
              <Plus className="h-4 w-4" />
            </Button>
          </div>
          <div className="mt-3 flex items-center gap-2">
            <Input
              value={channelSearch}
              onChange={(event) => setChannelSearch(event.target.value)}
              placeholder={bilingual('搜索频道、描述或最近消息', 'Search channels, descriptions, or recent messages')}
              className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none"
            />
            <Button
              variant="outline"
              className="h-9 shrink-0 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none"
              onClick={() => setCreateOpen(true)}
            >
              {bilingual('新建频道', 'Create channel')}
            </Button>
          </div>
          <div className="mt-4 flex items-center justify-between">
            <span className="collab-kicker">{bilingual('频道目录', 'Directory')}</span>
            <span className="collab-toolbar-pill collab-micro">
              {bilingual(`${filteredChannels.length} 个结果`, `${filteredChannels.length} results`)}
            </span>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-2.5 py-3">
          {loadingChannels ? (
            <div className="flex items-center justify-center p-6">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : channels.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground">{bilingual('还没有团队频道，先创建一个。', 'There are no team channels yet. Create one first.')}</div>
          ) : filteredChannels.length === 0 ? (
            <div className="border-b border-dashed border-border/70 px-4 py-6 text-center">
              <div className="text-[13px] font-semibold text-foreground">{bilingual('没有匹配的频道', 'No matching channels')}</div>
              <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                {bilingual('试试换个关键词，或者直接创建一个新的协作频道。', 'Try another keyword, or create a new collaboration channel directly.')}
              </div>
            </div>
          ) : (
            <div className="space-y-1">
              {filteredChannels.map((channel) => {
                const selected = channel.channel_id === selectedChannelId;
                const activity = channelLastActivity(channel);
                const activityLabel = activity ? formatDateTime(activity) : '';
                return (
                  <div key={channel.channel_id} className="group">
                    <button
                      type="button"
                      onClick={() => setSelectedChannelId(channel.channel_id)}
                      className={`collab-channel-card w-full px-2.5 py-2 text-left ${channel.muted ? 'text-muted-foreground' : ''}`}
                      data-selected={selected}
                    >
                      <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-2.5">
                        <div className="min-w-0">
                          <div className="flex items-center gap-1.5">
                            {channel.visibility === 'team_private' ? (
                              <Lock className="h-3 w-3 shrink-0 text-muted-foreground" />
                            ) : (
                              <Hash className="h-3 w-3 shrink-0 text-muted-foreground" />
                            )}
                            <div className="collab-channel-title min-w-0 flex-1 truncate">
                              {channel.name}
                            </div>
                            {channel.unread_count > 0 ? (
                              <span className={`shrink-0 rounded-full px-1.5 py-0.5 text-[10px] ${
                                channel.muted ? 'bg-muted text-muted-foreground' : 'bg-primary text-primary-foreground'
                              }`}>
                                {channel.unread_count}
                              </span>
                            ) : null}
                          </div>
                          {channel.description ? (
                            <div className="collab-meta mt-0.5 line-clamp-1 leading-4.5">
                              {channel.description}
                            </div>
                          ) : null}
                          <div className="mt-1 flex items-center gap-1.5 text-[10px] text-muted-foreground">
                            <span className="collab-toolbar-pill collab-micro shrink-0">
                              {channelVisibilityLabel(channel.visibility)}
                            </span>
                            {channel.pinned ? <Pin className="h-3 w-3 shrink-0 text-muted-foreground" /> : null}
                            {channel.muted ? <BellOff className="h-3 w-3 shrink-0 text-muted-foreground" /> : null}
                            {activityLabel ? (
                              <span className="collab-meta truncate">{activityLabel}</span>
                            ) : null}
                          </div>
                        </div>

                        <div className="flex min-w-[42px] items-start justify-end">
                          <div className={`flex items-center gap-0.5 transition-opacity ${selected ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}>
                            <button
                              type="button"
                              onClick={(event) => {
                                event.stopPropagation();
                                void handleTogglePinned(channel);
                              }}
                              className={`inline-flex h-6 w-6 items-center justify-center rounded-[8px] transition-colors hover:bg-accent ${
                                channel.pinned ? 'text-primary hover:text-primary' : 'text-muted-foreground hover:text-foreground'
                              }`}
                              title={channel.pinned ? bilingual('取消置顶', 'Unpin') : bilingual('置顶频道', 'Pin channel')}
                            >
                              {channel.pinned ? <PinOff className="h-3.5 w-3.5" /> : <Pin className="h-3.5 w-3.5" />}
                            </button>
                            <button
                              type="button"
                              onClick={(event) => {
                                event.stopPropagation();
                                void handleToggleMuted(channel);
                              }}
                              className={`inline-flex h-6 w-6 items-center justify-center rounded-[8px] transition-colors hover:bg-accent ${
                                channel.muted ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'
                              }`}
                              title={channel.muted ? bilingual('取消静音', 'Unmute') : bilingual('静音频道', 'Mute channel')}
                            >
                              <BellOff className="h-3.5 w-3.5" />
                            </button>
                          </div>
                        </div>
                      </div>
                    </button>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>

      <div className="flex min-w-0 flex-1 gap-4 overflow-hidden px-4 py-4">
        <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
          {channelDetail ? (
            <>
              {isMobile ? (
                <>
              <div className="border-b border-border/70 bg-background px-5 py-4">
                <div className="flex items-start justify-between gap-4">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <div className="truncate text-[18px] font-semibold tracking-[-0.02em] text-foreground">
                        {channelDetail.name}
                      </div>
                      <span className="rounded-full border border-border/70 px-2 py-0.5 text-[10px] text-muted-foreground">
                        {channelVisibilityLabel(channelDetail.visibility)}
                      </span>
                      <span className="rounded-full border border-border/70 px-2 py-0.5 text-[10px] text-muted-foreground">
                        {normalizeAgentDisplayName(channelDetail.default_agent_name)}
                      </span>
                      <span className="rounded-full border border-border/70 px-2 py-0.5 text-[10px] text-muted-foreground">
                        {bilingual(`${channelDetail.member_count} 位成员`, `${channelDetail.member_count} members`)}
                      </span>
                      <span
                        className="collab-autonomy-pill collab-autonomy-pill-mobile"
                        data-mode={currentAutonomyMeta.mode}
                      >
                        {currentAutonomyMeta.shortLabel}
                      </span>
                    </div>
                    <div className="mt-1 flex items-center gap-2">
                      <button
                        type="button"
                        onClick={() => setAutonomyGuideOpen((prev) => !prev)}
                        className="collab-autonomy-trigger"
                        aria-expanded={autonomyGuideOpen}
                      >
                        {bilingual('协作方式说明', 'Collaboration guide')}
                      </button>
                    </div>
                    {autonomyGuideOpen ? renderAutonomyGuide() : null}
                    {channelDetail.description ? (
                      <p className="mt-1.5 line-clamp-1 text-[11px] leading-5 text-muted-foreground">
                        {channelDetail.description}
                      </p>
                    ) : (
                      <p className="mt-1.5 text-[11px] leading-5 text-muted-foreground">
                        {bilingual('讨论在公共区发生，持续推进在协作项内完成。', 'Discussion happens in the public area, while sustained execution happens inside collaboration items.')}
                      </p>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5">
                    {isMobile ? (
                      <>
                        <Button
                          variant={sidePanelMode === 'documents' ? 'default' : 'outline'}
                          size="sm"
                          onClick={() => setSidePanelMode((current) => current === 'documents' ? null : 'documents')}
                          className="h-8 rounded-[12px] px-2.5 text-[11px] shadow-none"
                        >
                          {bilingual('资料', 'Docs')}
                        </Button>
                        {isCodingChannel ? (
                          <Button
                            variant={sidePanelMode === 'workspace' ? 'default' : 'outline'}
                            size="sm"
                            onClick={() => setSidePanelMode((current) => current === 'workspace' ? null : 'workspace')}
                            className="h-8 rounded-[12px] px-2.5 text-[11px] shadow-none"
                          >
                            {bilingual('工作区', 'Workspace')}
                          </Button>
                        ) : null}
                        <Button
                          variant={sidePanelMode === 'ai_outputs' ? 'default' : 'outline'}
                          size="sm"
                          onClick={() => setSidePanelMode((current) => current === 'ai_outputs' ? null : 'ai_outputs')}
                          className="h-8 rounded-[12px] px-2.5 text-[11px] shadow-none"
                        >
                          {bilingual('AI 产出', 'AI outputs')}
                        </Button>
                        <Button variant="ghost" size="sm" className="h-8 rounded-[12px] px-2.5 text-[11px]" onClick={() => setMembersOpen(true)}>
                          {bilingual('成员', 'Members')}
                        </Button>
                        <Button variant="ghost" size="sm" className="h-8 rounded-[12px] px-2.5 text-[11px]" onClick={() => setSettingsOpen(true)}>
                          {bilingual('频道设置', 'Channel settings')}
                        </Button>
                      </>
                    ) : (
                      <>
                        <Button
                          variant={inspectorTab ? 'default' : 'outline'}
                          size="sm"
                          onClick={() =>
                            setSidePanelMode((current) =>
                              current && current !== 'thread' && current !== 'workspace' ? null : 'documents',
                            )
                          }
                          className="h-8 rounded-[10px] border-border/70 px-3 text-[11px] shadow-none"
                        >
                          {bilingual('信息面板', 'Info panel')}
                        </Button>
                        {isCodingChannel ? (
                          <Button
                            variant={sidePanelMode === 'workspace' ? 'default' : 'outline'}
                            size="sm"
                            onClick={() => setSidePanelMode((current) => (current === 'workspace' ? null : 'workspace'))}
                            className="h-8 rounded-[10px] border-border/70 px-3 text-[11px] shadow-none"
                          >
                            {bilingual('工作区', 'Workspace')}
                          </Button>
                        ) : null}
                      </>
                    )}
                  </div>
                </div>
              </div>

              <div className="border-b border-border/70 bg-background px-5 py-2.5">
                <div className="flex flex-wrap items-center gap-4">
                  <div className="flex items-center gap-5">
                    {([
                      { key: 'work', label: bilingual('协作项', 'Collaboration') },
                      { key: 'update', label: bilingual('讨论区', 'Discussion') },
                    ] as Array<{ key: ChannelDisplayView; label: string }>).map((item) => (
                      <button
                        key={item.key}
                        type="button"
                        onClick={() => handleSelectSurfaceView(item.key)}
                        className={`border-b-2 px-0 py-1 text-[12px] font-medium transition-colors ${
                          surfaceView === item.key
                            ? 'border-foreground text-foreground'
                            : 'border-transparent text-muted-foreground hover:text-foreground'
                        }`}
                      >
                        {item.label}
                      </button>
                    ))}
                  </div>
                  {surfaceView === 'work' ? (
                    <div className="flex min-w-0 flex-1 flex-col gap-2 border-l border-border/60 pl-4">
                      <div className="flex flex-wrap items-center gap-3">
                        {COLLABORATION_SURFACE_FILTER_OPTIONS.map((item) => (
                          <button
                            key={item.key}
                            type="button"
                            onClick={() => setWorkSurfaceFilter(item.key)}
                            className={`inline-flex items-center gap-1.5 text-[11px] transition-colors ${
                              workSurfaceFilter === item.key
                                ? 'font-semibold text-foreground'
                                : 'text-muted-foreground hover:text-foreground'
                            }`}
                          >
                            <span>{item.label}</span>
                            <span className={`rounded-full px-1.5 py-0.5 text-[10px] ${
                              workSurfaceFilter === item.key
                                ? 'bg-foreground text-background'
                                : 'bg-muted/60 text-muted-foreground'
                            }`}>
                              {collaborationSurfaceCounts[item.key]}
                            </span>
                          </button>
                        ))}
                      </div>
                      <div className="flex flex-wrap items-center gap-3">
                        {COLLABORATION_STATUS_FILTER_OPTIONS.map((item) => (
                          <button
                            key={item.key}
                            type="button"
                            onClick={() => setWorkStatusFilter(item.key)}
                            className={`inline-flex items-center gap-1.5 text-[11px] transition-colors ${
                              workStatusFilter === item.key
                                ? 'font-semibold text-foreground'
                                : 'text-muted-foreground hover:text-foreground'
                            }`}
                          >
                            <span>{item.label}</span>
                            <span className={`rounded-full px-1.5 py-0.5 text-[10px] ${
                              workStatusFilter === item.key
                                ? 'bg-foreground text-background'
                                : 'bg-muted/60 text-muted-foreground'
                            }`}>
                              {collaborationStatusCounts[item.key]}
                            </span>
                          </button>
                        ))}
                      </div>
                    </div>
                  ) : null}
                </div>
              </div>
                </>
              ) : null}

              {!isMobile ? renderDesktopWorkspace() : (
                <>
              {isMobile && channelDetail.document_folder_path && !desktopThreadOpen ? (
                <div className="border-b border-border/60 bg-muted/[0.08] px-3 py-1.5">
                  <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
                    <button
                      type="button"
                      onClick={() => setSidePanelMode('documents')}
                      className={`rounded-full border px-2 py-0.5 transition-colors ${
                        sidePanelMode === 'documents'
                          ? 'border-foreground bg-foreground text-background'
                          : 'border-border/70 bg-background hover:text-foreground'
                      }`}
                    >
                      {bilingual(`资料 ${channelDocuments.length}`, `Docs ${channelDocuments.length}`)}
                    </button>
                    <button
                      type="button"
                      onClick={() => setSidePanelMode('ai_outputs')}
                      className={`rounded-full border px-2 py-0.5 transition-colors ${
                        sidePanelMode === 'ai_outputs'
                          ? 'border-foreground bg-foreground text-background'
                          : 'border-border/70 bg-background hover:text-foreground'
                      }`}
                    >
                      {bilingual(`AI 产出 ${channelAiOutputs.length}`, `AI outputs ${channelAiOutputs.length}`)}
                    </button>
                    {threadRootId && !isMobile ? (
                      <span className="rounded-full border border-border/70 bg-background px-2 py-0.5">
                        {bilingual('当前线程', 'Current thread')}
                      </span>
                    ) : null}
                    <span className="ml-auto hidden text-[10px] text-muted-foreground/75 xl:inline">
                      {bilingual('默认仅读已附加资料', 'Only attached docs are read by default')}
                    </span>
                  </div>
                </div>
              ) : null}

              {desktopThreadOpen ? (
                (() => {
                  const activeThread = threadRootMessage!;
                  return (
                <div className="flex min-h-0 flex-1 flex-col bg-background">
                  <div className="border-b border-border/70 px-5 py-3">
                    <div className="flex flex-wrap items-center justify-between gap-3">
                        <div className="min-w-0 flex flex-1 items-center gap-2.5">
                          <button
                            type="button"
                            onClick={closeThreadPanel}
                            className="inline-flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground transition-colors hover:text-foreground"
                        >
                          <ArrowLeft className="h-3.5 w-3.5" />
                          {bilingual('返回协作项列表', 'Back to collaboration list')}
                        </button>
                        <span className="hidden h-4 w-px shrink-0 bg-border/70 sm:inline-flex" />
                        <div className="min-w-0 flex-1 truncate text-[15px] font-semibold leading-6 text-foreground">
                          {currentThreadSummary}
                        </div>
                        <span className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${
                          collaborationSurfaceTone(activeThread.surface)
                        }`}>
                          {collaborationSurfaceLabel(activeThread.surface)}
                        </span>
                        <span className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${
                          workStatusTone(activeThread.display_status, activeThread.surface)
                        }`}>
                          {workStatusLabel(activeThread.display_status, activeThread.surface)}
                        </span>
                        {threadDelegationRuntime ? (
                          <span
                            className={`shrink-0 rounded-full px-2 py-0.5 text-[10px] ${delegationRuntimeStatusTone(
                              threadDelegationRuntime.status,
                            )}`}
                            title={buildDelegationRuntimeSummary(threadDelegationRuntime)}
                          >
                            {buildDelegationRuntimeSummary(threadDelegationRuntime)}
                          </span>
                        ) : null}
                      </div>
                      <div className="flex flex-wrap items-center justify-end gap-1.5">
                        <ThreadStatusBar
                          replyCount={currentThreadReplyCount}
                          documentCount={currentThreadDocumentCount}
                          aiOutputCount={currentThreadAiOutputCount}
                          compact
                        />
                        {!['adopted', 'rejected'].includes(activeThread.display_status || '') ? (
                          <>
                            <Button
                              type="button"
                              size="sm"
                              className="h-8 rounded-[10px] px-3 text-[11px] shadow-none"
                              onClick={() => requestAdoptThreadConfirm(activeThread)}
                            >
                              {bilingual('标记采用', 'Mark as adopted')}
                            </Button>
                            <details className="relative">
                              <summary className="flex h-8 cursor-pointer list-none items-center rounded-[10px] border border-border/70 bg-background px-2.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground">
                                <MoreHorizontal className="h-3.5 w-3.5" />
                              </summary>
                              <div className="absolute right-0 z-20 mt-1 min-w-[128px] rounded-[10px] border border-border/70 bg-background p-1 shadow-lg">
                                {activeThread.surface === 'temporary' ? (
                                  <button
                                    type="button"
                                    onClick={() => requestMarkThreadHighlightedConfirm(activeThread)}
                                    className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                  >
                                    {bilingual('升级为正式协作', 'Promote to formal collaboration')}
                                  </button>
                                ) : null}
                                {activeThread.display_status !== 'awaiting_confirmation' ? (
                                  <button
                                    type="button"
                                    onClick={() => requestAwaitingConfirmationConfirm(activeThread)}
                                    className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                  >
                                    {bilingual('等你判断', 'Needs your decision')}
                                  </button>
                                ) : null}
                                <button
                                  type="button"
                                  onClick={() => requestSyncThreadResultConfirm(activeThread)}
                                  className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                >
                                  {bilingual('同步结果', 'Sync result')}
                                </button>
                                <button
                                  type="button"
                                  onClick={() => requestArchiveThreadConfirm(activeThread)}
                                  className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                >
                                  {bilingual('标记未采用', 'Mark as rejected')}
                                </button>
                              </div>
                            </details>
                          </>
                        ) : null}
                      </div>
                    </div>
                    <div className="mt-2">
                      <button
                        type="button"
                        onClick={() => setThreadRootExpanded((prev) => !prev)}
                        className="flex w-full items-center gap-2 border-t border-border/60 pt-2.5 text-left"
                      >
                        <ChevronRight className={`h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform ${threadRootExpanded ? 'rotate-90' : ''}`} />
                        <div className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                  {bilingual('协作起点', 'Origin')} · {activeThread.author_name} · {bilingual('点击查看起点详情', 'Click to view origin details')}
                        </div>
                      </button>
                      {threadRootExpanded ? (
                        <div className="mt-2">
                          <ThreadRootCard message={activeThread} />
                        </div>
                      ) : null}
                    </div>
                  </div>
                  <div
                    ref={threadScrollRef}
                    className="min-h-0 flex-1 overflow-y-auto px-5 py-5"
                    onScroll={() => {
                      stickThreadToBottomRef.current = isNearBottom(threadScrollRef.current);
                    }}
                  >
                    <div className="w-full space-y-4">
                      {threadMessages.map((message) => (
                        <ThreadFlowMessage
                          key={message.message_id}
                          message={message}
                          isOwn={message.author_user_id === user?.id}
                          showFullDiagnostics={canViewFullDiagnostics}
                        />
                      ))}
                      <div ref={threadEndRef} />
                    </div>
                  </div>
                  <div className="border-t border-border/50 bg-background px-5 py-2.5">
                    <div className="w-full">
                      <div className="flex flex-wrap items-center gap-1.5 pb-2">
                        <div className="relative min-w-[160px] flex-1">
                          {renderComposerTargetTrigger(
                            'thread',
                            'h-8 rounded-[10px] border border-border/60 bg-background px-3 text-[11px] shadow-none',
                          )}
                        </div>
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => {
                            setCapabilityDetailKey(null);
                            setShowCapabilityPicker(true);
                          }}
                          className="h-8 rounded-[10px] border-border/60 bg-background px-2.5 text-[11px] shadow-none"
                        >
                {bilingual('技能', 'Skills')}
                        </Button>
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => setShowDocPicker(true)}
                          className="h-8 rounded-[10px] border-border/60 bg-background px-2.5 text-[11px] shadow-none"
                        >
                {bilingual('附件', 'Attachments')}
                        </Button>
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => fileInputRef.current?.click()}
                          disabled={uploadingDocument}
                          className="h-8 rounded-[10px] border-border/60 bg-background px-2.5 text-[11px] shadow-none"
                        >
                {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传', 'Upload')}
                        </Button>
                      </div>
                      {(attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                        <div className="flex flex-wrap items-center gap-1.5 pb-2">
                          {attachedDocs.map((doc) => (
                            <span
                              key={doc.id}
                                className="inline-flex max-w-full items-center gap-1 rounded-full bg-muted/[0.08] px-2.5 py-1 text-[11px] text-muted-foreground"
                            >
                              <Paperclip className="h-3 w-3 shrink-0" />
                              <span className="max-w-[160px] truncate">{doc.display_name || doc.name}</span>
                              <button
                                type="button"
                                onClick={() => {
                                  setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                                  setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                                }}
                                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                              >
                                <X className="h-3 w-3" />
                              </button>
                            </span>
                          ))}
                          {selectedCapabilities.map((item) => (
                            <span
                              key={item.key}
                                className="inline-flex max-w-full items-center gap-1 rounded-full bg-muted/[0.08] px-2.5 py-1 text-[11px] text-muted-foreground"
                            >
                              <Sparkles className="h-3 w-3 shrink-0" />
                              <span className="max-w-[160px] truncate">{item.name}</span>
                              <button
                                type="button"
                                onClick={() => removeCapabilityRef(item.ref)}
                                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
                              >
                                <X className="h-3 w-3" />
                              </button>
                            </span>
                          ))}
                        </div>
                      ) : null}
                      <div className="relative rounded-[10px] border border-border/50 bg-muted/[0.015] px-1.5 py-1.5">
                        <Textarea
                          ref={threadComposerTextareaRef}
                          value={threadComposeText}
                          onChange={handleThreadComposerChange}
                          onSelect={handleThreadComposerSelect}
                          onKeyDown={(event) => handleComposerMentionKeyDown('thread', event)}
                          className="min-h-[76px] resize-none border-0 bg-transparent px-2 py-1 pr-30 text-[12.5px] leading-5 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
                placeholder={bilingual('围绕这条线程继续推进…', 'Continue advancing this thread…')}
                        />
                        <div className="absolute bottom-2 right-2">
                          <Button
                            onClick={() => void handleSend(threadRootId)}
                            disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
                            className="h-8 rounded-[12px] px-3 text-[12px] shadow-none"
                          >
                            <MessageSquareReply className="mr-1 h-3.5 w-3.5" />
                            {bilingual('回复', 'Reply')}
                          </Button>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
                  );
                })()
              ) : (
                <>
                  <div
                    ref={messageScrollRef}
                    className="flex-1 overflow-y-auto py-3"
                    onScroll={() => {
                      stickMainToBottomRef.current = isNearBottom(messageScrollRef.current);
                    }}
                  >
                    {loadingMessages ? (
                      <div className="flex items-center justify-center py-10">
                        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                      </div>
                    ) : surfaceView === 'update' ? (
                      <div className="w-full space-y-4 px-4 md:px-6 lg:px-8 xl:px-10">
                        {discussionAreaMessages.length === 0 ? (
                          <div className="border-b border-dashed border-border/70 px-1 py-8 text-center">
                <div className="text-[12px] font-medium text-foreground">{bilingual('讨论区还没有内容', 'The discussion area is empty')}</div>
                            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                  {bilingual('这里用于团队交流、提醒、同步和 @成员，不默认进入协作过程。', 'Use this area for team communication, reminders, sync, and @mentions. It does not enter the collaboration flow by default.')}
                            </div>
                          </div>
                        ) : null}
                        {discussionTimelineEntries.map((entry) =>
                          entry.kind === 'day'
                            ? <DiscussionDayDivider key={entry.key} label={entry.label} />
                            : renderMessage(entry.message, entry.groupedWithPrevious),
                        )}
                        <div ref={messageEndRef} />
                      </div>
                    ) : (
                      <div className="flex w-full flex-col px-4 md:px-6 lg:px-8 xl:px-10 py-3">
                        <div className="mb-3 flex items-center justify-between gap-3 border-b border-border/70 pb-3">
                        <div>
                          <div className="text-[13px] font-semibold text-foreground">{bilingual('协作项', 'Collaboration item')}</div>
                          {workSurfaceFilter !== 'all' || workStatusFilter !== 'all' ? (
                            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                              {`${bilingual('当前筛选：', 'Current filter: ')}${workSurfaceFilter === 'all' ? bilingual('全部协作', 'All collaboration') : collaborationSurfaceFilterLabel(workSurfaceFilter)}${
                                workStatusFilter === 'all' ? '' : ` · ${collaborationStatusFilterLabel(workStatusFilter)}`
                              }`}
                            </div>
                          ) : null}
                          </div>
                          <div className="text-[11px] text-muted-foreground">
                            {bilingual(`${currentWorkOrUpdateItems.length} 条`, `${currentWorkOrUpdateItems.length} items`)}
                          </div>
                        </div>
                        <div className="overflow-hidden rounded-[12px] border border-border/70 bg-background">
                        {currentWorkOrUpdateItems.length === 0 ? (
                          <div className="px-6 py-10 text-center">
                            <div className="text-[14px] font-semibold text-foreground">
                              {collaborationWorklistTitle(workSurfaceFilter, workStatusFilter)}
                            </div>
                            <div className="mt-2 text-[12px] leading-6 text-muted-foreground">
                              {collaborationWorklistDescription(workSurfaceFilter, workStatusFilter)}
                            </div>
                          </div>
                        ) : (
                          currentWorkOrUpdateItems.map((item) => {
                            const selected = threadRootId === item.message.message_id;
                            return (
                              <button
                                key={item.message.message_id}
                                type="button"
                                onClick={() => void handleOpenThread(item.message)}
                                className={`w-full border-b border-border/60 px-4 py-3 text-left transition-colors last:border-b-0 ${
                                  selected
                                    ? 'border-foreground/20 bg-accent/25'
                                    : 'border-border/60 bg-background hover:bg-muted/[0.04]'
                                }`}
                              >
                                <div className="flex items-start justify-between gap-4">
                                  <div className="min-w-0 flex-1">
                                    <div className="flex flex-wrap items-center gap-2">
                                      <span className={`rounded-full px-2 py-0.5 text-[10px] ${
                                        collaborationSurfaceTone(item.message.surface)
                                      }`}>
                                        {collaborationSurfaceLabel(item.message.surface)}
                                      </span>
                                      <span className={`rounded-full px-2 py-0.5 text-[10px] ${
                                        workStatusTone(item.message.display_status, item.message.surface)
                                      }`}>
                                        {workStatusLabel(item.message.display_status, item.message.surface)}
                                      </span>
                                      <span className="rounded-full border border-border/60 px-2 py-0.5 text-[10px] text-muted-foreground">
                                        {item.message.source_kind === 'agent' ? bilingual('Agent 发起', 'Started by agent') : bilingual('成员发起', 'Started by member')}
                                      </span>
                                      <span className="ml-auto text-[10px] text-muted-foreground">
                                        {formatDateTime(item.message.created_at)}
                                      </span>
                                    </div>
                                    <div className="mt-2 truncate text-[14px] font-semibold text-foreground">
                                      {item.title}
                                    </div>
                                    <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                                      {item.preview || item.message.content_text}
                                    </div>
                                    {collaborationSurfaceHint(item.message.surface) ? (
                                      <div className="mt-1.5 collab-surface-hint">
                                        {collaborationSurfaceHint(item.message.surface)}
                                      </div>
                                    ) : null}
                                    <div className="mt-2 flex flex-wrap items-center gap-1.5 text-[10px]">
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                        {bilingual(`${item.message.reply_count} 条回复`, `${item.message.reply_count} replies`)}
                                      </span>
                                      {item.recentAgents.length > 0 ? (
                                        <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                          {item.recentAgents.slice(0, 2).join(' · ')}
                                        </span>
                                      ) : null}
                                      {item.documentCount > 0 ? (
                                        <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                          {bilingual(`${item.documentCount} 资料`, `${item.documentCount} docs`)}
                                        </span>
                                      ) : null}
                                      {item.aiOutputCount > 0 ? (
                                        <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                          {bilingual(`${item.aiOutputCount} 个 AI 产出`, `${item.aiOutputCount} AI outputs`)}
                                        </span>
                                      ) : null}
                                    </div>
                                  </div>
                                  <div className="shrink-0 pt-1 text-muted-foreground">
                                    <ChevronRight className="h-4 w-4" />
                                  </div>
                                </div>
                              </button>
                            );
                          })
                        )}
                      </div>
                    </div>
                  )}
                </div>

                  <div className="border-t border-border/70 bg-background px-5 py-3">
                    <input
                      ref={fileInputRef}
                      type="file"
                      accept={FILE_ACCEPT}
                      multiple
                      className="hidden"
                      onChange={(event) => void handleUploadToChannelFolder(event.target.files)}
                    />
                    <div className="w-full px-4 md:px-6 lg:px-8 xl:px-10">
                      <div className="flex flex-wrap items-center justify-between gap-2 pb-2">
                        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                        <div className="relative min-w-[180px] flex-1 sm:max-w-[220px] sm:flex-none">
                            {renderComposerTargetTrigger(
                              'root',
                              'h-8 rounded-[10px] border border-border/70 bg-background px-3 text-[11px] shadow-none',
                            )}
                          </div>
                          {isMobile ? (
                            <Button
                              variant="outline"
                              onClick={() => setComposerToolsOpen(true)}
                              className="h-8 rounded-[10px] border-border/70 bg-background px-2.5 text-[11px] shadow-none"
                            >
                              {bilingual('工具', 'Tools')}
                            </Button>
                          ) : (
                            <>
                              <Button
                                type="button"
                                variant="outline"
                                onClick={() => {
                                  setCapabilityDetailKey(null);
                                  setShowCapabilityPicker(true);
                                }}
                                className="h-8 rounded-[10px] border-border/70 bg-background px-2.5 text-[11px] shadow-none"
                              >
                              {bilingual('技能', 'Skills')}
                              </Button>
                              <Button
                                type="button"
                                variant="outline"
                                onClick={() => setShowDocPicker(true)}
                                className="h-8 rounded-[10px] border-border/70 bg-background px-2.5 text-[11px] shadow-none"
                              >
                              {bilingual('附件', 'Attachments')}
                              </Button>
                              <Button
                                type="button"
                                variant="outline"
                                onClick={() => fileInputRef.current?.click()}
                                disabled={uploadingDocument}
                                className="h-8 rounded-[10px] border-border/70 bg-background px-2.5 text-[11px] shadow-none"
                              >
                                {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传', 'Upload')}
                              </Button>
                            </>
                          )}
                        </div>
                        {!isMobile && (attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                          <div className="flex flex-wrap items-center justify-end gap-1.5">
                            {attachedDocs.length > 0 ? (
                              <span className="inline-flex items-center rounded-full bg-background/90 px-2.5 py-1 text-[10.5px] text-muted-foreground">
                                <Paperclip className="mr-1 h-3 w-3" />
                                {bilingual(`${attachedDocs.length} 个附件`, `${attachedDocs.length} attachment(s)`)}
                              </span>
                            ) : null}
                            {selectedCapabilities.length > 0 ? (
                              <span className="inline-flex items-center rounded-full bg-primary/[0.08] px-2.5 py-1 text-[10.5px] text-primary">
                                <Sparkles className="mr-1 h-3 w-3" />
                                {bilingual(`${selectedCapabilities.length} 个技能`, `${selectedCapabilities.length} skill(s)`)}
                              </span>
                            ) : null}
                          </div>
                        ) : null}
                      </div>

                      {(attachedDocs.length > 0 || selectedCapabilities.length > 0) && (
                        <div className="flex flex-wrap items-center gap-1.5 py-2">
                          {attachedDocs.map((doc) => (
                            <span
                              key={doc.id}
                              className="inline-flex max-w-full items-center gap-1 rounded-full bg-muted/[0.08] px-2.5 py-1 text-[11px] text-muted-foreground"
                            >
                              <Paperclip className="h-3 w-3 shrink-0" />
                              <span className="max-w-[180px] truncate">
                                {doc.display_name || doc.name}
                              </span>
                              <button
                                type="button"
                                onClick={() => {
                                  setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                                  setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                                }}
                                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                                title={bilingual('移除文档', 'Remove document')}
                              >
                                <X className="h-3 w-3" />
                              </button>
                            </span>
                          ))}
                          {selectedCapabilities.map((item) => (
                            <span
                              key={item.key}
                              className="inline-flex max-w-full items-center gap-1 rounded-full bg-muted/[0.08] px-2.5 py-1 text-[11px] text-muted-foreground"
                            >
                              <Sparkles className="h-3 w-3 shrink-0" />
                              <span className="max-w-[180px] truncate">{item.name}</span>
                              <button
                                type="button"
                                onClick={() => removeCapabilityRef(item.ref)}
                                className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
                                title={bilingual('移除能力', 'Remove capability')}
                              >
                                <X className="h-3 w-3" />
                              </button>
                            </span>
                          ))}
                        </div>
                      )}

                      <div className="relative rounded-[10px] border border-border/70 bg-background px-1.5 py-1.5">
                        <div className="px-2 pb-1 text-[10px] text-muted-foreground">
                          {rootStartsTemporaryCollaboration
                            ? `A temporary collaboration session will open around @${rootMentionedAgentLabel} first for discussion and clarification.`
                            : rootComposerIntent === 'work'
                              ? 'A collaboration item will be created and both AI and team members will continue driving this work.'
                              : 'This will go to discussion, which is best for human conversation, reminders, sync, and @mentions.'}
                        </div>
                        <Textarea
                          ref={rootComposerTextareaRef}
                          value={composeText}
                          onChange={handleRootComposerChange}
                          onSelect={handleRootComposerSelect}
                          onKeyDown={(event) => handleComposerMentionKeyDown('root', event)}
                          className="min-h-[76px] resize-none border-0 bg-transparent px-2 py-1 pr-28 text-[13px] leading-6 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
                          placeholder={
                            rootStartsTemporaryCollaboration
                              ? `Talk with @${rootMentionedAgentLabel} first to clarify the problem, context, and next step…`
                              : rootComposerIntent === 'work'
                                ? 'Start a collaboration item and explain what should be driven forward…'
                                : 'Send a discussion message to sync, remind, or @mention team members…'
                          }
                        />
                        <div className="absolute bottom-2 right-2 flex items-center gap-2">
                          {isMobile && (attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                            <span className="hidden rounded-full bg-muted/40 px-2 py-1 text-[10.5px] text-muted-foreground sm:inline-flex">
                              {attachedDocs.length > 0 ? bilingual(`${attachedDocs.length} 附件`, `${attachedDocs.length} attachment(s)`) : ''}
                              {attachedDocs.length > 0 && selectedCapabilities.length > 0 ? ' · ' : ''}
                              {selectedCapabilities.length > 0 ? bilingual(`${selectedCapabilities.length} 技能`, `${selectedCapabilities.length} skill(s)`) : ''}
                            </span>
                          ) : null}
                          <Button
                            onClick={() => void handleSend()}
                            disabled={sending || (!composeText.trim() && selectedCapabilityRefs.length === 0)}
                            className="h-8 rounded-[10px] px-3 text-[12px] shadow-none"
                          >
                            <Send className="mr-1 h-3.5 w-3.5" />
                            {rootStartsTemporaryCollaboration
                              ? bilingual('开始临时协作', 'Start temporary collaboration')
                              : rootComposerIntent === 'work'
                                ? bilingual('开始协作', 'Start collaboration')
                                : bilingual('发送到讨论区', 'Send to discussion')}
                          </Button>
                        </div>
                      </div>
                      {error ? <div className="pt-2 text-[12px] text-destructive">{error}</div> : null}
                    </div>
                  </div>
                </>
              )}
                </>
              )}
            </>
          ) : (
            <div className="collab-empty-shell flex flex-1 items-center justify-center px-8 text-center text-sm text-muted-foreground">
              <div>
                <div className="collab-section-title">{bilingual('选择一个团队频道进入工作台', 'Choose a team channel to enter the workspace')}</div>
                <div className="collab-meta mt-3 max-w-[420px] leading-6">
                  {bilingual('在这里打开讨论流、协作项、频道资料和 AI 产出，开始一条完整的协作路径。', 'Open discussions, collaboration items, channel docs, and AI outputs here to start a full collaboration flow.')}
                </div>
              </div>
            </div>
          )}
        </div>
        {renderDesktopWorkspacePanel()}
        {renderDesktopInspector()}
      </div>

      <BottomSheetPanel open={isMobile && sidePanelMode === 'thread' && !!threadRootId} onOpenChange={(open) => { if (!open) { closeThreadPanel(); } }} title={bilingual('协作项', 'Collaboration item')} description={bilingual('围绕这件事继续推进', 'Continue advancing this work')} fullHeight>
        <div className="space-y-4">
          {threadRootMessage ? (
            <>
              <div className="rounded-[18px] bg-background px-3 py-3 shadow-[inset_0_0_0_1px_rgba(15,23,42,0.06)]">
                <div className="flex items-center gap-2">
                  <div className="text-[10px] font-medium uppercase tracking-[0.08em] text-muted-foreground">
                    {bilingual('协作项', 'Collaboration item')}
                  </div>
                  <span className={`rounded-full px-2 py-0.5 text-[10px] ${
                    workStatusTone(threadRootMessage.display_status, threadRootMessage.surface)
                  }`}>
                    {workStatusLabel(threadRootMessage.display_status, threadRootMessage.surface)}
                  </span>
                </div>
                <div className="mt-1 line-clamp-1 text-[12px] font-medium leading-5 text-foreground">
                  {currentThreadSummary}
                </div>
                <div className="mt-2">
                  <ThreadStatusBar
                    replyCount={currentThreadReplyCount}
                    documentCount={currentThreadDocumentCount}
                    aiOutputCount={currentThreadAiOutputCount}
                  />
                </div>
                  {threadRuntime ? (
                    <div className="mt-2 rounded-[12px] border border-border/70 bg-background/80 px-3 py-2 text-[11px] text-muted-foreground">
                      {bilingual('线程现场：', 'Thread workspace: ')}{threadRuntime.thread_worktree_path || threadRuntime.workspace_path || bilingual('未绑定', 'Not bound')}
                      {threadRuntime.thread_branch ? ` · ${bilingual('分支：', 'Branch: ')}${threadRuntime.thread_branch}` : ''}
                    </div>
                  ) : null}
                {!['adopted', 'rejected'].includes(threadRootMessage.display_status || '') ? (
                  <div className="mt-3 flex flex-wrap items-center gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="h-8 rounded-[12px] px-3 text-[11px] shadow-none"
                      onClick={() => requestAdoptThreadConfirm(threadRootMessage)}
                    >
                      {bilingual('标记采用', 'Mark as adopted')}
                    </Button>
                    <details className="relative">
                      <summary className="flex h-8 cursor-pointer list-none items-center rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] text-muted-foreground transition-colors hover:text-foreground">
                        <MoreHorizontal className="h-3.5 w-3.5" />
                      </summary>
                      <div className="absolute right-0 z-20 mt-1 min-w-[132px] rounded-[10px] border border-border/70 bg-background p-1 shadow-lg">
                        {threadRootMessage.surface === 'temporary' ? (
                          <button
                            type="button"
                            onClick={() => requestMarkThreadHighlightedConfirm(threadRootMessage)}
                            className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                          >
                            {bilingual('升级为正式协作', 'Promote to formal collaboration')}
                          </button>
                        ) : null}
                        {threadRootMessage.display_status !== 'awaiting_confirmation' ? (
                          <button
                            type="button"
                            onClick={() => requestAwaitingConfirmationConfirm(threadRootMessage)}
                            className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                          >
                            {bilingual('等你判断', 'Needs your decision')}
                          </button>
                        ) : null}
                        <button
                          type="button"
                          onClick={() => requestSyncThreadResultConfirm(threadRootMessage)}
                          className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                        >
                          {bilingual('同步结果', 'Sync result')}
                        </button>
                        <button
                          type="button"
                          onClick={() => requestArchiveThreadConfirm(threadRootMessage)}
                          className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                        >
                          {bilingual('标记未采用', 'Mark as rejected')}
                        </button>
                      </div>
                    </details>
                  </div>
                ) : null}
              </div>
              <button
                type="button"
                onClick={() => setThreadRootExpanded((prev) => !prev)}
                className="flex w-full items-center gap-2 rounded-[12px] border border-border/70 bg-background px-3 py-2 text-left"
              >
                <ChevronRight className={`h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform ${threadRootExpanded ? 'rotate-90' : ''}`} />
                <div className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">
                  {bilingual('协作起点', 'Origin')} · {threadRootMessage.author_name} · {bilingual('点击查看起点详情', 'Click to view origin details')}
                </div>
              </button>
              {threadRootExpanded ? <ThreadRootCard message={threadRootMessage} /> : null}
            </>
          ) : null}
          {threadMessages.map((message) => (
            <ThreadFlowMessage
              key={message.message_id}
              message={message}
              isOwn={message.author_user_id === user?.id}
              showFullDiagnostics={canViewFullDiagnostics}
            />
          ))}
            <div className="rounded-[12px] border border-border/50 bg-muted/[0.015] p-2">
              <div className="flex flex-wrap items-center gap-1.5 px-1 pb-2">
                <div className="min-w-[160px] flex-1">
                  {renderComposerTargetTrigger(
                    'thread',
                    'h-8 rounded-[10px] border border-border/60 bg-background px-3 text-[11px] shadow-none',
                  )}
                </div>
              <Button
                variant="outline"
                onClick={() => setComposerToolsOpen(true)}
                className="h-8 rounded-[10px] border-border/60 bg-background px-2.5 text-[11px] shadow-none"
              >
                {bilingual('工具', 'Tools')}
              </Button>
            </div>
            {(attachedDocs.length > 0 || selectedCapabilities.length > 0) && (
              <div className="flex flex-wrap items-center gap-1.5 px-1 pb-2">
                {attachedDocs.map((doc) => (
                  <span
                    key={doc.id}
                    className="inline-flex max-w-full items-center gap-1 rounded-full bg-muted/[0.08] px-2.5 py-1 text-[11px] text-muted-foreground"
                  >
                    <Paperclip className="h-3 w-3 shrink-0" />
                    <span className="max-w-[160px] truncate">{doc.display_name || doc.name}</span>
                    <button
                      type="button"
                      onClick={() => {
                        setAttachedDocs((prev) => prev.filter((item) => item.id !== doc.id));
                        setPendingDocIds((prev) => prev.filter((id) => id !== doc.id));
                      }}
                      className="inline-flex h-4 w-4 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background hover:text-foreground"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </span>
                ))}
                {selectedCapabilities.map((item) => (
                  <span
                    key={item.key}
                    className="inline-flex max-w-full items-center gap-1 rounded-full bg-primary/[0.08] px-2.5 py-1 text-[11px] text-primary"
                  >
                    <Sparkles className="h-3 w-3 shrink-0" />
                    <span className="max-w-[160px] truncate">{item.name}</span>
                    <button
                      type="button"
                      onClick={() => removeCapabilityRef(item.ref)}
                      className="inline-flex h-4 w-4 items-center justify-center rounded-full text-primary/70 transition-colors hover:bg-background hover:text-primary"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </span>
                ))}
              </div>
            )}
            <div className="relative rounded-[10px] border border-border/50 bg-background px-1.5 py-1.5">
              <Textarea
                ref={threadComposerTextareaRef}
                value={threadComposeText}
                onChange={handleThreadComposerChange}
                onSelect={handleThreadComposerSelect}
                onKeyDown={(event) => handleComposerMentionKeyDown('thread', event)}
                className="min-h-[76px] resize-none border-0 bg-transparent px-2 py-1 pr-28 text-[12.5px] leading-5 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
                placeholder={bilingual('围绕这条消息继续推进…', 'Continue advancing this thread…')}
              />
              <div className="absolute bottom-2 right-2">
                <Button
                  onClick={() => void handleSend(threadRootId)}
                  disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
                  className="h-8 rounded-[12px] px-3 text-[12px] shadow-none"
                >
                  <MessageSquareReply className="mr-1 h-3.5 w-3.5" />{bilingual('回复', 'Reply')}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </BottomSheetPanel>

      <BottomSheetPanel
        open={isMobile && sidePanelMode === 'workspace'}
        onOpenChange={(open) => {
          if (!open) {
            setSidePanelMode(null);
          }
        }}
        title={bilingual('工作区面板', 'Workspace panel')}
        description={bilingual('查看当前编程项目频道绑定的项目空间、仓库主检出和线程现场。', 'Inspect the project space, main repo checkout, and thread workspace bound to the current coding channel.')}
        fullHeight
      >
        {renderWorkspacePanelContent()}
      </BottomSheetPanel>

      <BottomSheetPanel
        open={isMobile && (sidePanelMode === 'documents' || sidePanelMode === 'ai_outputs')}
        onOpenChange={(open) => {
          if (!open) {
            setSidePanelMode(null);
          }
        }}
        title={sidePanelMode === 'ai_outputs' ? bilingual('AI 产出', 'AI outputs') : bilingual('频道资料', 'Channel docs')}
        description={sidePanelMode === 'ai_outputs' ? bilingual('查看当前频道的 AI 草稿与结果', 'Inspect AI drafts and results in the current channel') : bilingual('查看当前频道的资料与文档目录', 'Inspect documents and the document directory for the current channel')}
        fullHeight
      >
        {sidePanelMode === 'ai_outputs' ? renderAiOutputsInspector() : renderDocumentsInspector()}
      </BottomSheetPanel>

      <BottomSheetPanel
        open={composerToolsOpen}
        onOpenChange={setComposerToolsOpen}
        title={bilingual('频道工具', 'Channel tools')}
        description={bilingual('把技能、文档和上传资料挂到当前频道消息里。', 'Attach skills, documents, and uploaded material to the current channel message.')}
      >
        <div className="space-y-2">
          <button
            type="button"
            onClick={() => {
              setComposerToolsOpen(false);
              setCapabilityDetailKey(null);
              setShowCapabilityPicker(true);
            }}
            className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
          >
            <Sparkles className="h-4 w-4 text-primary" />
            <div className="min-w-0">
                <div className="text-[13px] font-medium text-foreground">{bilingual('技能', 'Skills')}</div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                {bilingual('选择当前 Agent 真正可调用的技能和扩展。', 'Choose the skills and extensions that the current agent can actually call.')}
              </div>
            </div>
          </button>
          <button
            type="button"
            onClick={() => {
              setComposerToolsOpen(false);
              setShowDocPicker(true);
            }}
            className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
          >
            <Paperclip className="h-4 w-4 text-primary" />
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-foreground">{bilingual('附件', 'Attachments')}</div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                {bilingual('把团队文档附加到当前频道消息或当前线程；只有附加后的文档才会进入本轮上下文。', 'Attach team documents to the current channel message or thread. Only attached docs enter the current turn context.')}
              </div>
            </div>
          </button>
          <button
            type="button"
            onClick={() => {
              setComposerToolsOpen(false);
              fileInputRef.current?.click();
            }}
            disabled={uploadingDocument}
            className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30 disabled:opacity-50"
          >
            <Upload className="h-4 w-4 text-primary" />
            <div className="min-w-0">
              <div className="text-[13px] font-medium text-foreground">
                {uploadingDocument ? bilingual('上传中…', 'Uploading…') : bilingual('上传文件', 'Upload file')}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                {bilingual('直接上传到本频道的文档目录，并附加到当前消息；不会因为上传就自动全量读取频道资料。', 'Upload directly into this channel’s document directory and attach it to the current message. Uploading does not automatically read all channel documents.')}
              </div>
            </div>
          </button>
        </div>
      </BottomSheetPanel>

      <ChatCapabilityPicker
        open={showCapabilityPicker}
        onOpenChange={(open) => {
          setShowCapabilityPicker(open);
          if (!open) {
            setCapabilityDetailKey(null);
          }
        }}
        catalog={capabilityCatalog}
        loading={capabilityLoading}
        error={capabilityError}
        initialSelectedKeys={selectedCapabilityKeys}
        initialDetailKey={capabilityDetailKey}
        onInsert={applyCapabilitySelection}
      />

      <DocumentPicker
        teamId={teamId}
        open={showDocPicker}
        onClose={() => setShowDocPicker(false)}
        onSelect={attachDocsToComposer}
        selectedIds={pendingDocIds}
      />

      <BottomSheetPanel
        open={publishDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            closePublishDialog();
          }
        }}
        title={bilingual('发布到团队文档', 'Publish to team documents')}
        description={bilingual('把这份 AI 产出升级成团队正式文档，并选择展示名称与目标目录。', 'Promote this AI output into an official team document and choose its display name and target directory.')}
        fullHeight={!isMobile}
      >
        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-[12px] text-muted-foreground">{bilingual('展示名称', 'Display name')}</label>
            <Input
              value={publishName}
              onChange={(event) => setPublishName(event.target.value)}
              placeholder={bilingual('文档名称', 'Document name')}
            />
          </div>
          <div>
            <label className="mb-1 block text-[12px] text-muted-foreground">{bilingual('目标目录', 'Target directory')}</label>
            <Select value={publishFolderPath} onValueChange={setPublishFolderPath}>
              <SelectTrigger>
                <SelectValue placeholder={bilingual('选择目录', 'Choose a directory')} />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="/">{bilingual('根目录', 'Root')}</SelectItem>
                {flattenFolders(folderTree).map((folder) => (
                  <SelectItem key={folder.path} value={folder.path}>
                    {folder.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {foldersLoading ? (
              <div className="mt-2 text-[11px] text-muted-foreground">{bilingual('正在读取团队文档目录…', 'Loading the team document directory…')}</div>
            ) : null}
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={closePublishDialog}>
              {bilingual('取消', 'Cancel')}
            </Button>
            <Button
              onClick={() => void confirmPublishAiOutput()}
              disabled={!publishTargetDoc || publishingDocId === publishTargetDoc.id}
            >
              {publishTargetDoc && publishingDocId === publishTargetDoc.id ? bilingual('发布中…', 'Publishing…') : bilingual('确认发布', 'Confirm publish')}
            </Button>
          </div>
        </div>
      </BottomSheetPanel>

        <BottomSheetPanel open={createOpen} onOpenChange={setCreateOpen} title={bilingual('创建团队频道', 'Create team channel')} description={bilingual('公开频道对团队成员可见，私密频道只对指定成员可见。', 'Public channels are visible to team members. Private channels are visible only to selected members.')} fullHeight={!isMobile}>
          <div className="space-y-4">
            <Input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} placeholder={bilingual('频道名称', 'Channel name')} />
            <Textarea value={form.description} onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))} placeholder={bilingual('频道说明', 'Channel description')} className="min-h-[88px]" />
            <Select value={form.visibility} onValueChange={(value) => setForm((prev) => ({ ...prev, visibility: value as ChatChannelVisibility }))}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="team_public">{bilingual('公开频道', 'Public channel')}</SelectItem>
                <SelectItem value="team_private">{bilingual('私密频道', 'Private channel')}</SelectItem>
              </SelectContent>
            </Select>
            <Select value={form.channelType} onValueChange={(value) => setForm((prev) => ({ ...prev, channelType: value as ChatChannelType }))}>
              <SelectTrigger><SelectValue placeholder={bilingual('频道类型', 'Channel type')} /></SelectTrigger>
              <SelectContent>
                  <SelectItem value="general">{bilingual('普通协作频道', 'General collaboration channel')}</SelectItem>
                  <SelectItem value="coding">{bilingual('编程项目频道', 'Coding project channel')}</SelectItem>
              </SelectContent>
            </Select>
            <Select value={form.defaultAgentId} onValueChange={(value) => setForm((prev) => ({ ...prev, defaultAgentId: value }))}>
              <SelectTrigger><SelectValue placeholder={bilingual('默认 Agent', 'Default agent')} /></SelectTrigger>
              <SelectContent>
                {visibleAgents.map((agent) => (
                  <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {form.channelType === 'coding' ? (
              <>
                <div className="rounded-[12px] bg-muted/[0.05] px-3 py-2 text-[11px] leading-5 text-muted-foreground">
                  {bilingual('编程项目频道会自动创建并绑定一个项目工作区，后续协作线程默认围绕这个工作区和 thread worktree 执行。', 'Coding channels automatically create and bind a project workspace. Later collaboration threads run around this workspace and its thread worktree by default.')}
                </div>
                <Input
                  value={form.workspaceDisplayName}
                  onChange={(event) => setForm((prev) => ({ ...prev, workspaceDisplayName: event.target.value }))}
                  placeholder={bilingual('项目工作区名称', 'Project workspace name')}
                />
                <Input
                  value={form.repoDefaultBranch}
                  onChange={(event) => setForm((prev) => ({ ...prev, repoDefaultBranch: event.target.value }))}
                  placeholder={bilingual('默认分支（如 main）', 'Default branch (for example: main)')}
                />
              </>
            ) : (
              <div className="rounded-[12px] bg-muted/[0.05] px-3 py-2 text-[11px] leading-5 text-muted-foreground">
                {bilingual('普通协作频道不绑定项目工作区，适合文档、讨论和通用协作。', 'General collaboration channels do not bind a project workspace and are better for documents, discussion, and general collaboration.')}
              </div>
            )}
            {form.visibility === 'team_private' ? (
              <div className="space-y-2">
              <div className="text-[12px] font-medium text-foreground">{bilingual('初始成员', 'Initial members')}</div>
              <div className="max-h-[240px] space-y-2 overflow-y-auto rounded-[16px] border border-border/70 p-3">
                {teamMembers.map((member) => (
                  <label key={member.id} className="flex items-center gap-2 text-[12px]">
                    <input
                      type="checkbox"
                      checked={form.memberUserIds.includes(member.userId)}
                      onChange={(event) =>
                        setForm((prev) => ({
                          ...prev,
                          memberUserIds: event.target.checked
                            ? [...prev.memberUserIds, member.userId]
                            : prev.memberUserIds.filter((item) => item !== member.userId),
                        }))
                      }
                    />
                    <span>{member.displayName}</span>
                  </label>
                ))}
              </div>
            </div>
          ) : null}
          <div className="flex justify-end">
            <Button onClick={() => void handleCreateChannel()} disabled={!form.name.trim() || !form.defaultAgentId}>
              <Plus className="mr-1 h-4 w-4" />{bilingual('创建频道', 'Create channel')}
            </Button>
          </div>
        </div>
      </BottomSheetPanel>

        <BottomSheetPanel
          open={onboardingOpen}
          onOpenChange={setOnboardingOpen}
          title={bilingual('填写频道启动信息', 'Fill in channel kickoff details')}
          description={bilingual('这四项会直接写入频道设置和频道记忆，供协作线程长期参考。', 'These four fields are written directly into channel settings and channel memory for long-term collaboration reference.')}
          fullHeight={!isMobile}
        >
          <div className="space-y-4">
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('频道目标', 'Channel goal')}</div>
              <Textarea
                value={form.channelGoal}
                onChange={(event) => setForm((prev) => ({ ...prev, channelGoal: event.target.value }))}
                placeholder={bilingual('这个频道主要围绕什么事情建立？', 'What is this channel mainly created for?')}
                className="min-h-[76px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('参与人', 'Participants')}</div>
              <Textarea
                value={form.participantNotes}
                onChange={(event) => setForm((prev) => ({ ...prev, participantNotes: event.target.value }))}
                placeholder={bilingual('主要谁会参与协作？谁是关键判断人？', 'Who mainly participates? Who makes the key decisions?')}
                className="min-h-[76px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('产出物', 'Outputs')}</div>
              <Textarea
                value={form.expectedOutputs}
                onChange={(event) => setForm((prev) => ({ ...prev, expectedOutputs: event.target.value }))}
                placeholder={bilingual('最终希望形成什么结果、文档、方案或交付物？', 'What result, document, plan, or deliverable do you want in the end?')}
                className="min-h-[76px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('协作方式', 'Collaboration style')}</div>
              <Input
                value={form.collaborationStyle}
                onChange={(event) => setForm((prev) => ({ ...prev, collaborationStyle: event.target.value }))}
                placeholder={bilingual('例如：偏讨论 / 偏方案 / 偏执行 / 偏评审 / 混合', 'For example: discussion-heavy / planning-heavy / execution-heavy / review-heavy / mixed')}
              />
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="outline" onClick={() => setOnboardingOpen(false)}>
                {bilingual('稍后再填', 'Fill later')}
              </Button>
              <Button onClick={() => void handleSaveOnboarding()}>
                <Save className="mr-1 h-4 w-4" />
                {bilingual('写入频道信息', 'Save channel info')}
              </Button>
            </div>
          </div>
        </BottomSheetPanel>

        <BottomSheetPanel open={settingsOpen} onOpenChange={setSettingsOpen} title={bilingual('频道设置', 'Channel settings')} description={bilingual('维护频道名称、描述、可见范围和默认 Agent。', 'Maintain the channel name, description, visibility, and default agent.')} fullHeight={!isMobile}>
          <div className="space-y-4">
            <Input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} placeholder={bilingual('频道名称', 'Channel name')} />
            <Textarea value={form.description} onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))} placeholder={bilingual('频道说明', 'Channel description')} className="min-h-[88px]" />
            <Select value={form.visibility} onValueChange={(value) => setForm((prev) => ({ ...prev, visibility: value as ChatChannelVisibility }))}>
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="team_public">{bilingual('公开频道', 'Public channel')}</SelectItem>
                <SelectItem value="team_private">{bilingual('私密频道', 'Private channel')}</SelectItem>
              </SelectContent>
            </Select>
            <div className="space-y-2 rounded-[16px] border border-border/70 bg-muted/[0.04] px-4 py-3">
              <div className="text-[13px] font-medium text-foreground">{bilingual('频道类型', 'Channel type')}</div>
              <Select value={form.channelType} onValueChange={(value) => setForm((prev) => ({ ...prev, channelType: value as ChatChannelType }))}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="general">{bilingual('普通协作频道', 'General collaboration channel')}</SelectItem>
                  <SelectItem value="coding">{bilingual('编程项目频道', 'Coding project channel')}</SelectItem>
                </SelectContent>
              </Select>
              <div className="text-[11px] leading-5 text-muted-foreground">
                {bilingual('当前类型：', 'Current type: ')}{channelTypeLabel(form.channelType)}。
                {form.channelType === 'coding'
                  ? bilingual(' 保存后会确保频道绑定项目工作区，并让协作线程默认围绕代码现场执行。', ' After saving, the channel stays bound to the project workspace and collaboration threads run around the code context by default.')
                  : bilingual(' 保存后会解绑频道对项目工作区的使用关系，但不会删除底层 repo/worktree。', ' After saving, the channel is detached from the project workspace, but the underlying repo/worktree is not deleted.')}
              </div>
            </div>
            <Select value={form.defaultAgentId} onValueChange={(value) => setForm((prev) => ({ ...prev, defaultAgentId: value }))}>
              <SelectTrigger><SelectValue placeholder={bilingual('默认 Agent', 'Default agent')} /></SelectTrigger>
              <SelectContent>
                {visibleAgents.map((agent) => (
                  <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {form.channelType === 'coding' ? (
              <div className="space-y-3 rounded-[16px] border border-border/70 bg-muted/[0.04] px-4 py-3">
                <div>
                <div className="text-[13px] font-medium text-foreground">{bilingual('项目工作区', 'Project workspace')}</div>
                  <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                    {bilingual('这个频道会绑定一个项目工作区，所有协作线程都会从这里派生代码现场。', 'This channel binds to a project workspace. All collaboration threads derive their code context from it.')}
                  </div>
                </div>
                <div className="space-y-1">
                  <div className="text-[12px] text-muted-foreground">{bilingual('工作区名称', 'Workspace name')}</div>
                  <Input
                    value={form.workspaceDisplayName}
                    onChange={(event) => setForm((prev) => ({ ...prev, workspaceDisplayName: event.target.value }))}
                    placeholder={bilingual('项目工作区名称', 'Project workspace name')}
                  />
                </div>
                <div className="space-y-1">
                  <div className="text-[12px] text-muted-foreground">{bilingual('默认分支', 'Default branch')}</div>
                  <Input
                    value={form.repoDefaultBranch}
                    onChange={(event) => setForm((prev) => ({ ...prev, repoDefaultBranch: event.target.value }))}
                    placeholder="main"
                  />
                </div>
                {channelDetail?.workspace_path ? (
                  <div className="rounded-[12px] bg-background/80 px-3 py-2 text-[11px] text-muted-foreground">
                    {bilingual('工作区：', 'Workspace: ')}{channelDetail.workspace_display_name || channelDetail.name}
                    <br />
                    {bilingual('路径：', 'Path: ')}{channelDetail.workspace_path}
                    {channelDetail.repo_path ? (
                      <>
                        <br />
                        {bilingual('仓库：', 'Repo: ')}{channelDetail.repo_path}
                      </>
                    ) : null}
                    {channelDetail.main_checkout_path || channelDetail.repo_root ? (
                      <>
                        <br />
                        {bilingual('主检出：', 'Main checkout: ')}{channelDetail.main_checkout_path || channelDetail.repo_root}
                      </>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ) : (
              <div className="rounded-[16px] border border-border/70 bg-muted/[0.04] px-4 py-3 text-[11px] leading-5 text-muted-foreground">
                {bilingual('普通协作频道不展示项目工作区。若当前频道之前已经是编程项目频道，切回普通协作后只会解绑，不会删除底层代码现场。', 'General collaboration channels do not show a project workspace. If this channel was previously a coding channel, switching back will only detach it and will not delete the underlying repo or worktree.')}
              </div>
            )}
            {form.channelType === 'general' && channelDetail?.workspace_governance?.has_detached_workspace ? (
              <div className="space-y-3 rounded-[16px] border border-border/70 bg-muted/[0.04] px-4 py-3">
                <div>
                  <div className="text-[13px] font-medium text-foreground">{bilingual('解绑工作区治理', 'Detached workspace governance')}</div>
                  <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                    {bilingual('当前频道有一套已解绑但仍保留的项目工作区。可以恢复到编程项目频道，也可以单独归档或删除这套代码现场。', 'This channel has a detached project workspace that is still retained. You can restore it to a coding channel, archive it, or delete the code context separately.')}
                  </div>
                </div>
                <div className="rounded-[12px] bg-background/80 px-3 py-2 text-[11px] text-muted-foreground">
                  {bilingual('状态：', 'Status: ')}{workspaceLifecycleLabel(channelDetail.workspace_governance.lifecycle_state)}
                  <br />
                  {bilingual('工作区：', 'Workspace: ')}{channelDetail.workspace_governance.workspace_display_name || channelDetail.name}
                  {channelDetail.workspace_governance.workspace_path ? (
                    <>
                      <br />
                      {bilingual('路径：', 'Path: ')}{channelDetail.workspace_governance.workspace_path}
                    </>
                  ) : null}
                  {channelDetail.workspace_governance.repo_path ? (
                    <>
                      <br />
                      {bilingual('仓库：', 'Repo: ')}{channelDetail.workspace_governance.repo_path}
                    </>
                  ) : null}
                  {channelDetail.workspace_governance.main_checkout_path ? (
                    <>
                      <br />
                      {bilingual('主检出：', 'Main checkout: ')}{channelDetail.workspace_governance.main_checkout_path}
                    </>
                  ) : null}
                  {channelDetail.workspace_governance.retention_until ? (
                    <>
                      <br />
                      {bilingual('保留至：', 'Retained until: ')}{formatDateTime(channelDetail.workspace_governance.retention_until)}
                    </>
                  ) : null}
                </div>
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    size="sm"
                    onClick={() => void handleRestoreDetachedWorkspace()}
                    disabled={workspaceGovernanceAction !== null}
                  >
                    {workspaceGovernanceAction === 'restore' ? bilingual('恢复中…', 'Restoring…') : bilingual('恢复为编程频道', 'Restore as coding channel')}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() => void handleArchiveDetachedWorkspace()}
                    disabled={workspaceGovernanceAction !== null}
                  >
                    {workspaceGovernanceAction === 'archive' ? bilingual('归档中…', 'Archiving…') : bilingual('归档工作区', 'Archive workspace')}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={() => void handleDeleteDetachedWorkspace()}
                    disabled={workspaceGovernanceAction !== null}
                  >
                    {workspaceGovernanceAction === 'delete' ? bilingual('删除中…', 'Deleting…') : bilingual('删除工作区', 'Delete workspace')}
                  </Button>
                </div>
              </div>
            ) : null}
          <div className="space-y-1">
            <div className="text-[12px] text-muted-foreground">{bilingual('管理 Agent 主动度', 'Manager agent autonomy')}</div>
            <Select
              value={form.agentAutonomyMode}
              onValueChange={(value) =>
                setForm((prev) => ({ ...prev, agentAutonomyMode: value as ChatChannelAgentAutonomyMode }))
              }
            >
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="standard">{bilingual('标准模式', 'Standard mode')}</SelectItem>
                <SelectItem value="proactive">{bilingual('主动推进模式', 'Proactive mode')}</SelectItem>
                <SelectItem value="agent_lead">{bilingual('Agent 主导模式', 'Agent-led mode')}</SelectItem>
              </SelectContent>
            </Select>
            <div className="text-[11px] leading-5 text-muted-foreground">
              {getChannelAutonomyMeta(form.agentAutonomyMode).summary}
            </div>
          </div>
          <div className="space-y-3 rounded-[16px] border border-border/70 bg-muted/[0.04] px-4 py-3">
            <div>
              <div className="text-[13px] font-medium text-foreground">{bilingual('频道记忆', 'Channel memory')}</div>
              <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                {bilingual('管理 Agent 会长期参考这里的信息来判断建议、总结、提醒和推进方向。', 'The managing agent will use this information over time when deciding suggestions, summaries, reminders, and direction.') }
              </div>
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('频道目标', 'Channel goal')}</div>
              <Textarea
                value={form.channelGoal}
                onChange={(event) => setForm((prev) => ({ ...prev, channelGoal: event.target.value }))}
                placeholder={bilingual('这个频道主要围绕什么事情建立？', 'What is this channel mainly created for?')}
                className="min-h-[72px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('参与人', 'Participants')}</div>
              <Textarea
                value={form.participantNotes}
                onChange={(event) => setForm((prev) => ({ ...prev, participantNotes: event.target.value }))}
                placeholder={bilingual('主要谁会参与协作？谁是关键判断人？', 'Who mainly participates in the collaboration? Who makes the key decisions?')}
                className="min-h-[72px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('产出物', 'Expected outputs')}</div>
              <Textarea
                value={form.expectedOutputs}
                onChange={(event) => setForm((prev) => ({ ...prev, expectedOutputs: event.target.value }))}
                placeholder={bilingual('最终希望形成什么结果、文档、方案或交付物？', 'What result, document, plan, or deliverable do you want in the end?')}
                className="min-h-[72px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">{bilingual('协作方式', 'Collaboration style')}</div>
              <Input
                value={form.collaborationStyle}
                onChange={(event) => setForm((prev) => ({ ...prev, collaborationStyle: event.target.value }))}
                placeholder={bilingual('例如：偏讨论 / 偏方案 / 偏执行 / 偏评审 / 混合', 'For example: discussion-heavy / planning-heavy / execution-heavy / review-heavy / mixed')}
              />
            </div>
            {channelDetail?.orchestrator_state?.last_heartbeat_at ? (
              <div className="rounded-[12px] bg-background/80 px-3 py-2 text-[11px] text-muted-foreground">
                {bilingual('最近心跳：', 'Latest heartbeat: ')}{formatDateTime(channelDetail.orchestrator_state.last_heartbeat_at)}
                {channelDetail.orchestrator_state.last_heartbeat_reason
                  ? ` · ${channelDetail.orchestrator_state.last_heartbeat_reason}`
                  : ''}
              </div>
            ) : null}
          </div>
          <div className="flex justify-end">
            <Button onClick={() => void handleSaveChannel()}>
              <Save className="mr-1 h-4 w-4" />{bilingual('保存频道设置', 'Save channel settings')}
            </Button>
          </div>
          <div className="rounded-[16px] border border-destructive/20 bg-destructive/[0.04] px-4 py-3">
              <div className="text-[13px] font-medium text-foreground">{bilingual('删除频道', 'Delete channel')}</div>
            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                {bilingual('可以选择只删除频道，保留所有文档；或者彻底删除频道和相关文档。', 'You can delete only the channel and keep all documents, or permanently delete the channel together with related documents.')}
            </div>
            <div className="mt-3 flex justify-end">
              <Button variant="destructive" onClick={() => setDeleteDialogOpen(true)}>
                {bilingual('删除频道', 'Delete channel')}
              </Button>
            </div>
          </div>
        </div>
      </BottomSheetPanel>

      <ConfirmDialog
        open={Boolean(pendingCollaborationAction)}
        onOpenChange={(open) => {
          if (!open && !confirmingCollaborationAction) {
            setPendingCollaborationAction(null);
          }
        }}
        title={pendingCollaborationAction?.title || bilingual('确认操作', 'Confirm action')}
        description={pendingCollaborationAction?.description}
        confirmText={pendingCollaborationAction?.confirmText}
        variant={pendingCollaborationAction?.variant || 'default'}
        onConfirm={() => void handleConfirmCollaborationAction()}
        loading={confirmingCollaborationAction}
        cancelText={bilingual('取消', 'Cancel')}
      >
        {pendingCollaborationAction ? (
          <div className="space-y-3">
            <div
              className={`rounded-[16px] border px-3.5 py-3 ${
                pendingCollaborationAction.variant === 'destructive'
                  ? 'border-destructive/20 bg-destructive/[0.04]'
                  : 'border-border/60 bg-muted/[0.05]'
              }`}
            >
              <div className="flex items-center gap-2">
                <span
                  className={`inline-flex rounded-full px-2 py-0.5 text-[10px] font-medium ${
                    pendingCollaborationAction.variant === 'destructive'
                      ? 'bg-destructive/[0.12] text-destructive'
                      : 'bg-primary/[0.08] text-primary'
                  }`}
                >
                  {pendingCollaborationAction.actionLabel}
                </span>
                <div className="min-w-0 text-[12px] font-medium text-foreground">
                  <span className="truncate">{pendingCollaborationAction.subject}</span>
                </div>
              </div>
              {pendingCollaborationAction.note ? (
                <div className="mt-2 text-[11px] leading-5 text-muted-foreground">
                  {pendingCollaborationAction.note}
                </div>
              ) : null}
            </div>
          </div>
        ) : null}
      </ConfirmDialog>

      <ConfirmDialog
        open={deleteDialogOpen}
        onOpenChange={setDeleteDialogOpen}
        title={bilingual('删除频道', 'Delete channel')}
        description={bilingual('选择删除方式。这个动作无法撤销。', 'Choose a deletion mode. This action cannot be undone.')}
        confirmText={deleteMode === 'full_delete' ? bilingual('确认彻底删除', 'Confirm permanent delete') : bilingual('确认删除并保留文档', 'Confirm delete and keep documents')}
        variant="destructive"
        onConfirm={() => void handleDeleteChannel()}
        loading={deletingChannel}
      >
        <div className="space-y-2">
          <button
            type="button"
            onClick={() => setDeleteMode('preserve_documents')}
            className={`w-full rounded-[14px] border px-3 py-3 text-left transition-colors ${
              deleteMode === 'preserve_documents'
                ? 'border-foreground bg-accent/30'
                : 'border-border/70 hover:bg-accent/20'
            }`}
          >
            <div className="text-[13px] font-medium text-foreground">{bilingual('保留所有文档删除', 'Delete and keep all documents')}</div>
            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
              {bilingual('删除频道、消息、成员和线程，但保留频道资料与 AI 产出文档。', 'Delete the channel, messages, members, and threads, but keep channel documents and AI output docs.')}
            </div>
          </button>
          <button
            type="button"
            onClick={() => setDeleteMode('full_delete')}
            className={`w-full rounded-[14px] border px-3 py-3 text-left transition-colors ${
              deleteMode === 'full_delete'
                ? 'border-destructive/60 bg-destructive/[0.08]'
                : 'border-border/70 hover:bg-destructive/[0.05]'
            }`}
          >
            <div className="text-[13px] font-medium text-foreground">{bilingual('彻底删除', 'Permanent delete')}</div>
            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
              {bilingual('删除频道、消息、成员、线程，以及当前频道目录下的资料和该频道来源的 AI 产出。', 'Delete the channel, messages, members, threads, the documents in this channel directory, and AI outputs generated from this channel.')}
            </div>
          </button>
        </div>
      </ConfirmDialog>

      <BottomSheetPanel open={membersOpen} onOpenChange={setMembersOpen} title={bilingual('频道成员', 'Channel members')} description={bilingual('公开频道对团队成员可见；私密频道需要显式维护成员。', 'Public channels are visible to the whole team; private channels require explicit membership management.')} fullHeight={!isMobile}>
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <Select value={newMemberId} onValueChange={setNewMemberId}>
              <SelectTrigger><SelectValue placeholder={bilingual('选择成员', 'Select member')} /></SelectTrigger>
              <SelectContent>
                {memberOptions.map((member) => (
                  <SelectItem key={member.id} value={member.userId}>{member.displayName}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select value={newMemberRole} onValueChange={(value) => setNewMemberRole(value as 'member' | 'manager')}>
              <SelectTrigger className="w-[132px]"><SelectValue /></SelectTrigger>
              <SelectContent>
                  <SelectItem value="member">{bilingual('成员', 'Member')}</SelectItem>
                  <SelectItem value="manager">{bilingual('管理者', 'Manager')}</SelectItem>
              </SelectContent>
            </Select>
            <Button variant="outline" onClick={() => void handleAddMember()} disabled={!newMemberId}>
              {bilingual('添加', 'Add')}
            </Button>
          </div>
          <div className="space-y-2">
            {members.map((member) => (
              <div key={member.user_id} className="flex items-center gap-2 rounded-[16px] border border-border/70 px-3 py-2">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-foreground">
                    {teamMembers.find((item) => item.userId === member.user_id)?.displayName || member.user_id}
                  </div>
                  <div className="text-[11px] text-muted-foreground">{member.user_id}</div>
                </div>
                <Select value={member.role} onValueChange={(value) => void handleUpdateMemberRole(member.user_id, value as 'owner' | 'manager' | 'member')}>
                  <SelectTrigger className="w-[132px]"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="owner">Owner</SelectItem>
                    <SelectItem value="manager">Manager</SelectItem>
                    <SelectItem value="member">Member</SelectItem>
                  </SelectContent>
                </Select>
                <Button variant="ghost" size="sm" onClick={() => void handleRemoveMember(member.user_id)}>
                    {bilingual('移除', 'Remove')}
                </Button>
              </div>
            ))}
          </div>
        </div>
      </BottomSheetPanel>
    </div>
  );
}


