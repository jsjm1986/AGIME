import { useCallback, useEffect, useMemo, useRef, useState, type ChangeEvent, type CSSProperties, type KeyboardEvent, type ReactNode, type SyntheticEvent } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { createPortal } from 'react-dom';
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
  type ChatChannelDisplayKind,
  type ChatChannelDisplayStatus,
  type ChatChannelUserPrefs,
  type ChatChannelDeleteMode,
  type ChatChannelMember,
  type ChatChannelMessage,
  type ChatChannelMessageSurface,
  type ChatChannelSummary,
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

type ChannelRenderMessage = ChatChannelMessage & {
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  isStreaming?: boolean;
};

type ChannelDisplayView = 'work' | 'update';
type CollaborationStatusFilter = 'all' | 'proposed' | 'active' | 'awaiting_confirmation' | 'adopted' | 'rejected';
type CollaborationSurfaceFilter = 'all' | 'temporary' | 'issue';
type InspectorTabKey = 'documents' | 'ai_outputs' | 'members' | 'settings';

interface TeamChannelsPanelProps {
  teamId: string;
  initialChannelId?: string | null;
  initialThreadRootId?: string | null;
}

interface ChannelFormState {
  name: string;
  description: string;
  visibility: ChatChannelVisibility;
  defaultAgentId: string;
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

const CAPABILITY_BLOCK_HEADER = '请优先使用以下能力完成本轮任务：';
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
      return '草稿';
    case 'report':
      return '报告';
    case 'summary':
      return '总结';
    case 'review':
      return '审查';
    case 'plan':
      return '计划';
    case 'research':
      return '研究';
    case 'artifact':
      return '产物';
    case 'code':
      return '代码';
    default:
      return '其他';
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
    defaultAgentId,
    agentAutonomyMode: 'standard',
    channelGoal: '',
    participantNotes: '',
    expectedOutputs: '',
    collaborationStyle: '',
    memberUserIds: [],
  };
}

function channelVisibilityLabel(visibility: ChatChannelVisibility) {
  return visibility === 'team_private' ? '私密频道' : '公开频道';
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
    return '这条协作项';
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
    label: '标准模式',
    shortLabel: '标准模式',
    summary: 'Agent 主要负责提醒、建议和总结，不主动主导协作推进。',
  },
  {
    mode: 'proactive',
    label: '主动推进模式',
    shortLabel: '主动推进',
    summary: 'Agent 会更积极提醒、建议，并推动协作项启动。',
  },
  {
    mode: 'agent_lead',
    label: 'Agent 主导模式',
    shortLabel: 'Agent 主导',
    summary: 'Agent 可以主动创建协作项并推动讨论，但正式发布仍需人工确认。',
  },
];

const COLLABORATION_SURFACE_FILTER_OPTIONS: Array<{
  key: CollaborationSurfaceFilter;
  label: string;
  tone: 'neutral' | 'temporary' | 'issue';
}> = [
  { key: 'all', label: '全部', tone: 'neutral' },
  { key: 'temporary', label: '临时协作', tone: 'temporary' },
  { key: 'issue', label: '正式协作', tone: 'issue' },
];

const COLLABORATION_STATUS_FILTER_OPTIONS: Array<{
  key: CollaborationStatusFilter;
  label: string;
  tone: 'neutral' | 'idea' | 'progress' | 'decision' | 'success' | 'muted';
}> = [
  { key: 'all', label: '全部', tone: 'neutral' },
  { key: 'proposed', label: '建议', tone: 'idea' },
  { key: 'active', label: '推进中', tone: 'progress' },
  { key: 'awaiting_confirmation', label: '等你判断', tone: 'decision' },
  { key: 'adopted', label: '已采用', tone: 'success' },
  { key: 'rejected', label: '未采用', tone: 'muted' },
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
  if (status === 'proposed') return '建议';
  if (status === 'awaiting_confirmation') return '等你判断';
  if (status === 'adopted') return '已采用';
  if (status === 'rejected') return '未采用';
  if (status === 'active') return '推进中';
  if (fallbackSurface === 'activity') return '讨论';
  return '推进中';
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
  if (surface === 'temporary') return '临时协作';
  if (surface === 'issue') return '正式协作';
  if (surface === 'activity') return '讨论';
  return '协作';
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
  if (filter === 'temporary') return '临时协作';
  if (filter === 'issue') return '正式协作';
  return '全部协作';
}

function collaborationSurfaceHint(surface?: ChatChannelMessageSurface | null): string {
  if (surface === 'temporary') {
    return '先聊明白，确认后升级为正式协作';
  }
  if (surface === 'issue') {
    return '已进入正式推进，静默一段时间后会同步阶段进展到讨论区';
  }
  return '';
}

function collaborationStatusFilterLabel(filter: CollaborationStatusFilter): string {
  if (filter === 'proposed') return '建议';
  if (filter === 'active') return '推进中';
  if (filter === 'awaiting_confirmation') return '等你判断';
  if (filter === 'adopted') return '已采用';
  if (filter === 'rejected') return '未采用';
  return '全部';
}

function collaborationWorklistTitle(
  surfaceFilter: CollaborationSurfaceFilter,
  statusFilter: CollaborationStatusFilter,
): string {
  if (surfaceFilter === 'all' && statusFilter === 'all') {
    return '还没有协作项';
  }
  if (surfaceFilter === 'all') {
    return `还没有${collaborationStatusFilterLabel(statusFilter)}的协作项`;
  }
  if (statusFilter === 'all') {
    return `还没有${collaborationSurfaceFilterLabel(surfaceFilter)}`;
  }
  return `还没有${collaborationStatusFilterLabel(statusFilter)}的${collaborationSurfaceFilterLabel(surfaceFilter)}`;
}

function collaborationWorklistDescription(
  surfaceFilter: CollaborationSurfaceFilter,
  statusFilter: CollaborationStatusFilter,
): string {
  if (surfaceFilter === 'temporary' && statusFilter === 'all') {
    return '这里集中显示先聊明白、补充上下文和试探方向的临时协作。它们还没有被提炼成正式协作项。';
  }
  if (surfaceFilter === 'issue' && statusFilter === 'all') {
    return '这里集中显示已经进入正式推进的协作项，适合查看重点工作、结果和后续判断。';
  }
  if (surfaceFilter === 'all' && statusFilter === 'all') {
    return '先在讨论模式把事情说清楚，或者直接在下面新建一条协作项。';
  }
  if (surfaceFilter === 'all') {
    return '换个状态筛选看看，或者直接新建一条新的协作项。';
  }
  return `当前筛选的是${collaborationSurfaceFilterLabel(surfaceFilter)}，可以换个状态看看，或者先回到全部协作。`;
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
      label: '总结卡',
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
      label: '提醒卡',
      icon: TriangleAlert,
      bubbleClass: 'bg-amber-500/8',
      chipClass: 'bg-amber-500/10 text-amber-800',
      iconClass: 'bg-amber-100/60 text-amber-800',
    };
  }
  if (cardPurpose === 'formal_collaboration_progress_sync') {
    return {
      label: '进度卡',
      icon: MessageSquareReply,
      bubbleClass: 'bg-indigo-500/8',
      chipClass: 'bg-indigo-500/10 text-indigo-800',
      iconClass: 'bg-indigo-100/60 text-indigo-800',
    };
  }
  if (message.display_kind === 'suggestion') {
    return {
      label: '建议卡',
      icon: Lightbulb,
      bubbleClass: 'bg-sky-500/8',
      chipClass: 'bg-sky-500/10 text-sky-800',
      iconClass: 'bg-sky-100/60 text-sky-800',
    };
  }
  if (message.display_kind === 'result') {
    return {
      label: '结果卡',
      icon: CheckCheck,
      bubbleClass: 'bg-emerald-500/8',
      chipClass: 'bg-emerald-500/10 text-emerald-800',
      iconClass: 'bg-emerald-100/60 text-emerald-800',
    };
  }
  return {
    label: 'AI 回答',
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
  if (status === 'completed') return '已完成';
  if (status === 'blocked') return '阻塞';
  if (status === 'failed') return '失败';
  return status || '执行详情';
}

function runtimeDiagnosticFriendlySummary(diagnostics: RuntimeDiagnosticsMetadata) {
  if (diagnostics.status === 'completed') {
    return '系统已记录这轮执行情况，可按需查看详细处理记录。';
  }
  return '系统记录到这轮协作未完整结束。可以补充背景、资料或更明确的目标后再试。';
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
    author_name: user?.displayName || '我',
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
            协作
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
            {replyCount} 条回复
          </span>
          {documentCount > 0 ? (
            <span className="shrink-0 text-[10px] text-muted-foreground">
              {documentCount} 资料
            </span>
          ) : null}
          {aiOutputCount > 0 ? (
            <span className="shrink-0 text-[10px] text-primary">
              {aiOutputCount} 个 AI 产出
            </span>
          ) : null}
          <span className="min-w-0 flex-1 truncate text-[10px] text-muted-foreground">
            {compactPreview || '打开线程查看完整回复'}
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
                    升级为正式协作
                  </button>
                ) : null}
                {onArchive ? (
                  <button
                    type="button"
                    onClick={onArchive}
                    className="inline-flex items-center rounded-full bg-background/70 px-2 py-0.5 text-[10px] font-medium text-muted-foreground"
                  >
                    <Archive className="mr-1 h-3 w-3" />
                    标记未采用
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
        <DiscussionAvatar kind="user" label={isOwn ? '你' : message.author_name} />
      )}
      <div className={`flex min-w-0 max-w-[96%] flex-col ${isOwn ? 'items-end md:max-w-[92%] xl:max-w-[82%]' : 'items-start md:max-w-[92%] xl:max-w-[84%]'}`}>
        {!groupedWithPrevious ? (
          <div className="mb-1 flex items-center gap-1.5 px-1 text-[11px] text-muted-foreground">
            <span className="font-medium text-foreground">{isOwn ? '你' : message.author_name}</span>
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
        <DiscussionAvatar kind="user" label={isOwn ? '你' : message.author_name} />
      )}
      <div className={`flex min-w-0 max-w-[96%] flex-col ${isOwn ? 'items-end md:max-w-[92%] xl:max-w-[82%]' : 'items-start md:max-w-[92%] xl:max-w-[84%]'}`}>
        {!groupedWithPrevious ? (
          <div className="mb-1 flex items-center gap-1.5 px-1 text-[11px] text-muted-foreground">
            <span className="font-medium text-foreground">{isOwn ? '你' : message.author_name}</span>
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
            发起内容
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
          <span>处理说明</span>
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
        <span>执行详情</span>
        {diagnostics.status ? (
          <span className="collab-diagnostic-status">
            {runtimeDiagnosticStatusLabel(diagnostics.status)}
          </span>
        ) : null}
      </summary>
      <div className="collab-diagnostic-body">
        {diagnostics.summary ? (
          <div className="collab-diagnostic-line">
            <span className="collab-diagnostic-label">摘要</span>
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
        {replyCount} 条回复
      </span>
      {documentCount > 0 ? (
        <span className="collab-thread-stat" data-compact={compact ? 'true' : undefined}>
          {documentCount} 份资料
        </span>
      ) : null}
      {aiOutputCount > 0 ? (
        <span className="collab-thread-stat" data-compact={compact ? 'true' : undefined}>
          {aiOutputCount} 个 AI 产出
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
        <span className="font-medium uppercase tracking-[0.08em]">协作起点</span>
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
  const [threadRootExpanded, setThreadRootExpanded] = useState(false);
  const [members, setMembers] = useState<ChatChannelMember[]>([]);
  const [teamMembers, setTeamMembers] = useState<TeamMember[]>([]);
  const [channelDocuments, setChannelDocuments] = useState<DocumentSummary[]>([]);
  const [loadingChannelDocuments, setLoadingChannelDocuments] = useState(false);
  const [channelAiOutputs, setChannelAiOutputs] = useState<DocumentSummary[]>([]);
  const [loadingChannelAiOutputs, setLoadingChannelAiOutputs] = useState(false);
  const [sidePanelMode, setSidePanelMode] = useState<'documents' | 'ai_outputs' | 'members' | 'settings' | 'thread' | null>(null);
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
  const [membersOpen, setMembersOpen] = useState(false);
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
      return '围绕这件事继续协作';
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
      setError('当前无法读取团队频道，请稍后再试。');
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
      setError('当前无法读取频道详情，请稍后再试。');
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
      subtitle: `成员 · ${member.displayName || member.userId}`,
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
          ? `${item.label}${item.mention_type === 'agent' ? '（Agent）' : '（成员）'}`
          : item.label,
    }));
  }, [selectableRecipientMembers, visibleAgents]);

  const rootComposerPickerOptions = useMemo<ComposerPickerOption[]>(() => mentionOptions.map((option) => ({
    key: option.key,
    value: option.key,
    kind: option.mention_type,
    label: `${option.mention_type === 'agent' ? 'AI' : '成员'} · ${option.insertLabel}`,
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
      return '@成员 / Agent';
    }
    return `${latest.mention_type === 'agent' ? 'AI' : '成员'} · ${latest.insertLabel}`;
  }, [composeText, mentionOptions]);

  const threadComposerTargetLabel = useMemo(() => {
    const latest = findLatestMentionOptionInText(threadComposeText, mentionOptions);
    if (!latest) {
      return '@成员 / Agent';
    }
    return `${latest.mention_type === 'agent' ? 'AI' : '成员'} · ${latest.insertLabel}`;
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
          label: `${option.mention_type === 'agent' ? 'AI' : '成员'} · ${option.insertLabel}`,
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
        setCapabilityError('当前无法读取可用技能和扩展，请稍后再试。');
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
        default_agent_id: form.defaultAgentId,
        member_user_ids: form.visibility === 'team_private' ? form.memberUserIds : [],
      });
      setCreateOpen(false);
      resetCreateForm();
      await loadChannels();
      setSelectedChannelId(created.channel_id);
    } catch (createError) {
      console.error('Failed to create channel:', createError);
      setError('创建频道失败，请稍后再试。');
    }
  }, [form, loadChannels, resetCreateForm, teamId]);

  const handleSaveChannel = useCallback(async () => {
    if (!channelDetail) return;
    try {
      const updated = await chatApi.updateChannel(channelDetail.channel_id, {
        name: form.name,
        description: form.description || null,
        visibility: form.visibility,
        default_agent_id: form.defaultAgentId,
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
      setError('保存频道设置失败，请稍后再试。');
    }
  }, [channelDetail, form]);

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
      setChannelDetail(null);
      setMessages([]);
      setMembers([]);
      setChannelDocuments([]);
      setChannelAiOutputs([]);
      setSelectedChannelId(null);
      await loadChannels();
      addToast(
        'success',
        deleteMode === 'full_delete' ? '频道已彻底删除' : '频道已删除，文档已保留',
      );
    } catch (deleteError) {
      console.error('Failed to delete channel:', deleteError);
      setError(deleteMode === 'full_delete' ? '彻底删除频道失败，请稍后再试。' : '删除频道失败，请稍后再试。');
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
      setError('添加成员失败，请稍后再试。');
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
        setError('更新成员角色失败，请稍后再试。');
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
        setError('移除成员失败，请稍后再试。');
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
      setError('只有临时协作可以升级为正式协作。');
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
      addToast('success', '已升级为正式协作');
    } catch (promoteError) {
      console.error('Failed to mark work thread highlighted:', promoteError);
      setError('升级为正式协作失败，请稍后再试。');
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
      setError('更新协作项状态失败，请稍后再试。');
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
      setError('同步结果到讨论区失败，请稍后再试。');
    }
  }, [addToast, channelDetail, currentRootFilter.display_kind, currentRootFilter.display_status, loadChannel]);

  const handleArchiveThread = useCallback(async (message: ChannelRenderMessage) => {
    void handleUpdateThreadStatus(message, 'rejected', '协作项已标记为未采用');
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
        setError('当前卡片没有可继续推进的内容。');
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
        setError('开始协作失败，请稍后再试。');
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
            ? '已拒绝建议，关联协作项已移入拒绝分类'
            : '已拒绝建议，已移入拒绝分类',
        );
      } catch (rejectError) {
        console.error('Failed to reject suggestion card:', rejectError);
        setError('拒绝建议失败，请稍后再试。');
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
      title: '升级为正式协作',
      description: '把这条临时协作升级为正式协作。',
      confirmText: '确认升级',
      actionLabel: '升级为正式协作',
      subject,
      note: '升级后它会进入正式协作分类，作为需要持续推进的重点工作。',
      onConfirm: () => handleMarkThreadHighlighted(message),
    });
  }, [handleMarkThreadHighlighted, requestCollaborationActionConfirm]);

  const requestArchiveThreadConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: '标记未采用',
      description: '把这条协作项移入未采用分类。',
      confirmText: '确认未采用',
      actionLabel: '未采用',
      subject,
      note: '这不会删除内容，只是把它从当前推进流转到未采用分类。',
      variant: 'destructive',
      onConfirm: () => handleArchiveThread(message),
    });
  }, [handleArchiveThread, requestCollaborationActionConfirm]);

  const requestAwaitingConfirmationConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: '标记等判断',
      description: '把这条协作项交回你来判断下一步。',
      confirmText: '确认标记',
      actionLabel: '等判断',
      subject,
      note: '适合当前需要人工拍板、决定采纳或继续推进的情况。',
      onConfirm: () =>
        handleUpdateThreadStatus(message, 'awaiting_confirmation', '已标记为等你判断'),
    });
  }, [handleUpdateThreadStatus, requestCollaborationActionConfirm]);

  const requestAdoptThreadConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: '标记采用',
      description: '把这条协作结果正式标记为已采用。',
      confirmText: '确认采用',
      actionLabel: '已采用',
      subject,
      note: '采用后可继续在 AI 产出中整理并发布到团队文档。',
      onConfirm: () =>
        handleUpdateThreadStatus(message, 'adopted', '协作结果已标记为已采用'),
    });
  }, [handleUpdateThreadStatus, requestCollaborationActionConfirm]);

  const requestSyncThreadResultConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: '同步到讨论',
      description: '把当前结果同步回讨论区，便于团队成员查看。',
      confirmText: '确认同步',
      actionLabel: '同步到讨论',
      subject,
      note: '同步后会在讨论区生成结果卡，方便团队继续跟进。',
      onConfirm: () => handleSyncThreadResult(message),
    });
  }, [handleSyncThreadResult, requestCollaborationActionConfirm]);

  const requestStartCollaborationConfirm = useCallback((message: ChannelRenderMessage) => {
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
      title: '开始协作',
      description: '基于这张建议卡创建一条新的协作项。',
      confirmText: '确认开始',
      actionLabel: '开始协作',
      subject,
      note: '创建后会直接进入协作模式，围绕这件事继续推进。',
      onConfirm: () => handleStartCollaborationFromCard(message),
    });
  }, [handleStartCollaborationFromCard, requestCollaborationActionConfirm]);

  const requestRejectSuggestionConfirm = useCallback((message: ChannelRenderMessage) => {
    const subject = summarizeCollaborationSubject(message);
    requestCollaborationActionConfirm({
      title: '拒绝建议',
      description: '把这张建议卡移入拒绝分类。',
      confirmText: '确认拒绝',
      actionLabel: '拒绝建议',
      subject,
      note: '如果它关联了协作项，关联协作项也会一起进入拒绝分类。',
      variant: 'destructive',
      onConfirm: () => handleRejectSuggestionCard(message),
    });
  }, [handleRejectSuggestionCard, requestCollaborationActionConfirm]);

  const handleSend = useCallback(
    async (
      threadTarget?: string | null,
      submitMode?: 'discussion' | 'work' | 'execution',
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
        const message = '一条讨论消息只能 @ 一个 Agent。请只保留一个 Agent 后再发送。';
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
      const executionAgent =
        mentionedAgent ||
        visibleAgents.find((agent) => agent.id === resolvedComposerAgentId) ||
        visibleAgents.find((agent) => agent.id === channelDetail.default_agent_id) ||
        null;
      const selectedAgent =
        executionAgent;
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
      const requiresAiExecution = threadTarget ? true : rootSurface !== 'activity';
      const interactionMode = requiresAiExecution
        ? (submitMode === 'execution' ? 'execution' as const : 'conversation' as const)
        : undefined;
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
      const shouldEnterCollaborationWorkspace = !threadTarget && requiresAiExecution;
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
      } else if (requiresAiExecution) {
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
            interaction_mode: interactionMode,
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
            interaction_mode: interactionMode,
            agent_id: requiresAiExecution ? (executionAgent?.id || channelDetail.default_agent_id) : null,
            parent_message_id: null,
            attached_document_ids: pendingDocIdsSnapshot,
            mentions: rootMentions,
          });
          setAttachedDocs([]);
          setPendingDocIds([]);
          setSelectedCapabilityRefs([]);
          if (!requiresAiExecution || !response.streaming) {
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
        setError('发送频道消息失败，请稍后再试。');
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
      defaultAgentId: channelDetail.default_agent_id,
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
        setError('上传到频道文档目录失败，请稍后再试。');
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
      setError('加入频道资料失败，请稍后再试。');
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
      setError('发布到团队文档失败，请稍后再试。');
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
      if (message.author_type === 'agent' || message.display_kind === 'suggestion' || message.display_kind === 'result') {
          const linkedCollaborationId =
            typeof message.metadata?.linked_collaboration_id === 'string'
              ? message.metadata.linked_collaboration_id
              : null;
          const cardPurpose = message.metadata?.card_purpose as string | undefined;
          const primaryActionLabel = linkedCollaborationId
            ? '查看协作项'
            : cardPurpose === 'discussion_summary' || message.display_kind === 'suggestion'
              ? '开始协作'
              : null;
          const secondaryActionLabel =
            cardPurpose === 'discussion_summary' || message.display_kind === 'suggestion'
              ? '快速拒绝'
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
                <div className="collab-inspector-section-title">频道资料</div>
                <span className="collab-inspector-count">{channelDocuments.length}</span>
              </div>
              <div className="collab-inspector-section-meta">
                已上传到频道目录的资料，需要按需附加到当前消息或线程。
              </div>
            </div>
          </div>
          <div className="collab-inspector-meta-rail">
            <span className="collab-inspector-meta-label">目录</span>
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
                打开文档
              </Button>
            ) : null}
            <Button
              variant="outline"
              size="sm"
              className="h-8 rounded-[10px] px-2.5 text-[12px]"
              onClick={() => fileInputRef.current?.click()}
              disabled={uploadingDocument}
            >
              {uploadingDocument ? '上传中…' : '上传资料'}
            </Button>
          </div>
        </section>
        <div className="collab-inspector-list">
          {loadingChannelDocuments ? (
            <div className="collab-inspector-empty">正在读取频道资料…</div>
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
                      附加
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 rounded-[10px] px-2.5 text-[12px]"
                      onClick={() => openChannelDocuments(channelDetail.document_folder_path)}
                    >
                      打开
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className="collab-inspector-empty">这个频道还没有资料。先上传到频道目录，再按需附加到当前消息。</div>
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
                <div className="collab-inspector-section-title">AI 产出</div>
                <span className="collab-inspector-count">{channelAiOutputs.length}</span>
              </div>
              <div className="collab-inspector-section-meta">
                频道里的草稿、总结和结果。先附加使用，再决定是否加入频道资料或发布。
              </div>
            </div>
          </div>
        </section>
        <div className="collab-inspector-list">
        {loadingChannelAiOutputs ? (
          <div className="collab-inspector-empty">正在读取 AI 产出…</div>
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
                      {doc.source_thread_root_id ? '来自当前频道某条线程' : '来自当前频道'}
                    </div>
                  </div>
                  {doc.source_thread_root_id ? (
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-8 rounded-[10px] px-2.5 text-[12px]"
                      onClick={() => void loadThread(channelDetail.channel_id, doc.source_thread_root_id!)}
                    >
                      来源线程
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
                    附加
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[10px] px-2.5 text-[12px]"
                    onClick={() => void handlePromoteAiOutputToChannelDocs(doc)}
                    disabled={promotingDocId === doc.id || doc.folder_path === channelDetail.document_folder_path}
                  >
                    {doc.folder_path === channelDetail.document_folder_path
                      ? '已在频道资料'
                      : promotingDocId === doc.id
                        ? '处理中…'
                        : '加入频道资料'}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-8 rounded-[10px] px-2.5 text-[12px]"
                    onClick={() => void openPublishDialog(doc)}
                    disabled={publishingDocId === doc.id || doc.status === 'accepted'}
                  >
                    {doc.status === 'accepted' ? '已发布' : '发布到团队文档'}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="collab-inspector-empty">当前频道还没有 AI 产出。让 Agent 生成草稿、总结或报告后，这里会自动出现。</div>
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
              <div className="collab-inspector-section-title">成员管理</div>
              <span className="collab-inspector-count">{members.length}</span>
            </div>
            <div className="collab-inspector-section-meta">公开频道默认可见；私密频道需要手动维护成员与角色。</div>
          </div>
        </div>
        <div className="collab-inspector-member-add">
          <Select value={newMemberId} onValueChange={setNewMemberId}>
            <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
              <SelectValue placeholder="选择成员" />
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
                <SelectItem value="member">成员</SelectItem>
                <SelectItem value="manager">管理者</SelectItem>
              </SelectContent>
            </Select>
            <Button
              variant="outline"
              className="h-9 rounded-[12px] px-3 text-[12px]"
              onClick={() => void handleAddMember()}
              disabled={!newMemberId}
            >
              添加成员
            </Button>
          </div>
        </div>
        <div className="collab-inspector-helper">先选成员，再设角色并添加。成员标识默认截断显示，悬停可查看完整 ID。</div>
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
                <SelectItem value="owner">所有者</SelectItem>
                <SelectItem value="manager">管理者</SelectItem>
                <SelectItem value="member">成员</SelectItem>
              </SelectContent>
            </Select>
            <Button
              variant="ghost"
              size="sm"
              className="h-8 rounded-[10px] px-2.5 text-[12px]"
              onClick={() => void handleRemoveMember(member.user_id)}
            >
              移除
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
            <div className="collab-inspector-section-title">基本信息</div>
            <div className="collab-inspector-section-meta">维护频道名称、说明、可见范围与默认 Agent。</div>
          </div>
        </div>
        <div className="space-y-3">
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">频道名称</span>
            <Input
              value={form.name}
              onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
              placeholder="频道名称"
              className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
            />
          </label>
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">频道说明</span>
            <Textarea
              value={form.description}
              onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))}
              placeholder="频道说明"
              className="min-h-[92px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
            />
          </label>
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">可见范围</span>
            <Select value={form.visibility} onValueChange={(value) => setForm((prev) => ({ ...prev, visibility: value as ChatChannelVisibility }))}>
              <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
                <SelectValue />
              </SelectTrigger>
        <SelectContent>
          <SelectItem value="team_public">公开频道</SelectItem>
          <SelectItem value="team_private">私密频道</SelectItem>
        </SelectContent>
            </Select>
          </label>
          <label className="collab-inspector-field">
            <span className="collab-inspector-field-label">默认 Agent</span>
      <Select value={form.defaultAgentId} onValueChange={(value) => setForm((prev) => ({ ...prev, defaultAgentId: value }))}>
        <SelectTrigger className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none">
          <SelectValue placeholder="默认 Agent" />
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
            <div className="collab-inspector-section-title">协作策略</div>
            <div className="collab-inspector-section-meta">设置管理 Agent 在这个频道里的主动度和推进方式。</div>
          </div>
        </div>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">Agent 主动度</span>
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
            <SelectItem value="standard">标准模式</SelectItem>
            <SelectItem value="proactive">主动推进模式</SelectItem>
            <SelectItem value="agent_lead">Agent 主导模式</SelectItem>
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
            <div className="collab-inspector-section-title">频道记忆</div>
            <div className="collab-inspector-section-meta">管理 Agent 会长期参考这里的目标、参与人和产出物。</div>
          </div>
        </div>
        <div className="space-y-3">
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">频道目标</span>
          <Textarea
            value={form.channelGoal}
            onChange={(event) => setForm((prev) => ({ ...prev, channelGoal: event.target.value }))}
            placeholder="这个频道主要围绕什么事情建立？"
            className="min-h-[76px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">参与人</span>
          <Textarea
            value={form.participantNotes}
            onChange={(event) => setForm((prev) => ({ ...prev, participantNotes: event.target.value }))}
            placeholder="主要谁会参与协作？谁是关键判断人？"
            className="min-h-[76px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">产出物</span>
          <Textarea
            value={form.expectedOutputs}
            onChange={(event) => setForm((prev) => ({ ...prev, expectedOutputs: event.target.value }))}
            placeholder="最终希望形成什么结果、文档、方案或交付物？"
            className="min-h-[76px] rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        <label className="collab-inspector-field">
          <span className="collab-inspector-field-label">协作方式</span>
          <Input
            value={form.collaborationStyle}
            onChange={(event) => setForm((prev) => ({ ...prev, collaborationStyle: event.target.value }))}
            placeholder="例如：偏讨论 / 偏方案 / 偏执行 / 偏评审 / 混合"
            className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] text-[12px] shadow-none"
          />
        </label>
        {channelDetail?.orchestrator_state?.last_heartbeat_at ? (
          <div className="collab-inspector-inline-note">
            最近心跳：{formatDateTime(channelDetail.orchestrator_state.last_heartbeat_at)}
            {channelDetail.orchestrator_state.last_heartbeat_reason
              ? ` · ${channelDetail.orchestrator_state.last_heartbeat_reason}`
              : ''}
          </div>
        ) : null}
        </div>
      </section>
      <div className="flex justify-end">
        <Button className="h-9 rounded-[12px] px-3 text-[12px]" onClick={() => void handleSaveChannel()}>
          <Save className="mr-1 h-4 w-4" />保存频道设置
        </Button>
      </div>
      <section className="collab-inspector-danger">
        <div className="collab-inspector-section-title">危险操作</div>
        <div className="collab-inspector-section-meta">
          可以选择只删除频道，保留所有文档；或者彻底删除频道和相关文档。
        </div>
        <div className="mt-3 flex justify-end">
          <Button variant="destructive" className="h-9 rounded-[12px] px-3 text-[12px]" onClick={() => setDeleteDialogOpen(true)}>
            删除频道
          </Button>
        </div>
      </section>
    </div>
  );

  const renderDesktopInspector = () => {
    if (!channelDetail || isMobile) return null;
    const inspectorTab = sidePanelMode && sidePanelMode !== 'thread' ? sidePanelMode : null;
    if (!inspectorTab) return null;
    const inspectorTabs: Array<{ key: InspectorTabKey; label: string; count?: number; summary: string }> = [
      {
        key: 'documents',
        label: '资料',
        count: channelDocuments.length,
        summary: '频道资料目录与已上传材料。只有附加到当前消息或线程的内容才会进入当前上下文。',
      },
      {
        key: 'ai_outputs',
        label: 'AI 产出',
        count: channelAiOutputs.length,
        summary: 'Agent 在当前频道生成的草稿、总结和结果，可附加、归档或发布。',
      },
      {
        key: 'members',
        label: '成员',
        count: members.length,
        summary: '维护谁能看到这个频道、谁负责判断，以及各成员在频道中的角色。',
      },
      {
        key: 'settings',
        label: '设置',
        summary: '管理频道基本信息、Agent 主动度、频道记忆与删除策略。',
      },
    ];
    const activeTab = inspectorTabs.find((item) => item.key === inspectorTab) || inspectorTabs[0];
    return (
      <div className="collab-inspector-shell w-[368px] shrink-0">
        <div className="flex h-full min-h-0 flex-col">
          <div className="collab-shell-header collab-inspector-header px-4 py-4">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex-1">
                <div className="collab-inspector-kicker">信息面板</div>
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
        setError('更新频道置顶状态失败，请稍后再试。');
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
        setError('更新频道静音状态失败，请稍后再试。');
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
      const titleSource = message.content_text?.trim() || preview || '未命名协作项';
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

  const inspectorTab = (sidePanelMode && sidePanelMode !== 'thread' ? sidePanelMode : null) as InspectorTabKey | null;

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
              <span className="collab-kicker">{isDiscussionMode ? '讨论模式' : '新建协作项'}</span>
              <span className="collab-meta">
                {isDiscussionMode
                  ? (rootStartsTemporaryCollaboration
                      ? `已检测到 @${rootMentionedAgentLabel}，发送后会打开临时协作会话`
                      : '先商量，再决定是否进入协作')
                  : '这里只发送明确要推进的具体工作'}
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
            技能
          </Button>
          <Button
            type="button"
            variant="outline"
            className="h-8 rounded-[10px] px-2.5 text-[11px]"
            onClick={() => setShowDocPicker(true)}
          >
            附件
          </Button>
          <Button
            type="button"
            variant="outline"
            className="h-8 rounded-[10px] px-2.5 text-[11px]"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploadingDocument}
          >
            {uploadingDocument ? '上传中…' : '上传'}
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
                  title="移除文档"
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
                  title="移除能力"
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
                    ? `和 @${rootMentionedAgentLabel} 先聊清楚问题、背景和下一步…`
                    : '把问题、判断、同步或 @成员发到讨论区…')
                : '描述这条要推进的具体工作、预期结果和下一步…'
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
                  {rootStartsTemporaryCollaboration ? '开始临时协作' : '发送讨论'}
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
                  {rootStartsTemporaryCollaboration ? '开始临时协作' : '发到讨论区'}
                </Button>
                {!rootStartsTemporaryCollaboration ? (
                  <Button
                    onClick={() => void handleSend(undefined, 'work')}
                    disabled={composerDisabled}
                    className="h-8 rounded-[10px] px-3 text-[11px]"
                  >
                    <Sparkles className="mr-1 h-3.5 w-3.5" />
                    创建协作项
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
          <span className="collab-kicker">协作模式</span>
          <span className="collab-meta">围绕当前协作项继续补充、修正和推进</span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-[180px]">
            {renderComposerTargetTrigger(
              'thread',
              'h-8 rounded-[10px] border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none',
            )}
          </div>
          <Button type="button" variant="outline" className="h-8 rounded-[10px] px-2.5 text-[11px]" onClick={() => { setCapabilityDetailKey(null); setShowCapabilityPicker(true); }}>
            技能
          </Button>
          <Button type="button" variant="outline" className="h-8 rounded-[10px] px-2.5 text-[11px]" onClick={() => setShowDocPicker(true)}>
            附件
          </Button>
          <Button type="button" variant="outline" className="h-8 rounded-[10px] px-2.5 text-[11px]" onClick={() => fileInputRef.current?.click()} disabled={uploadingDocument}>
            {uploadingDocument ? '上传中…' : '上传'}
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
            placeholder="围绕这条协作线程继续对话、补充判断或澄清下一步…"
          />
          <div className="mt-0.5 flex items-center justify-end gap-2 px-1 pt-0.5">
            <Button
              type="button"
              variant="outline"
              onClick={() => void handleSend(threadRootId, 'execution')}
              disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
              className="h-8 rounded-[10px] px-3 text-[11px]"
            >
              <Sparkles className="mr-1 h-3.5 w-3.5" />
              执行这一步
            </Button>
            <Button
              onClick={() => void handleSend(threadRootId)}
              disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
              className="h-8 rounded-[10px] px-3 text-[11px]"
            >
              <MessageSquareReply className="mr-1 h-3.5 w-3.5" />
              发送对话
            </Button>
        </div>
      </div>
      {error ? <div className="pt-2 text-[12px] text-destructive">{error}</div> : null}
    </div>
  );

  const renderAutonomyGuide = () => (
    <div className="collab-autonomy-guide">
      <div className="collab-autonomy-guide-title">协作方式说明</div>
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
                <span className="collab-autonomy-guide-current">当前频道</span>
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
                <span className="collab-kicker shrink-0">智能协作频道</span>
                <h1 className="collab-display-title min-w-0 truncate">
                  {channelDetail.name}
                </h1>
                {channelDetail.is_processing ? (
                  <span className="collab-toolbar-pill collab-micro shrink-0" data-active="true">
                    处理中
                  </span>
                ) : null}
              </div>
              <Button
                variant={inspectorTab ? 'default' : 'outline'}
                size="sm"
                className="h-8 shrink-0 rounded-[10px] px-2.5 text-[11px]"
                onClick={() =>
                  setSidePanelMode((current) =>
                    current && current !== 'thread' ? null : 'documents',
                  )
                }
              >
                信息面板
              </Button>
            </div>

            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              <span className="collab-toolbar-pill collab-micro">
                {channelVisibilityLabel(channelDetail.visibility)}
              </span>
              <span className="collab-toolbar-pill collab-micro">
                {normalizeAgentDisplayName(channelDetail.default_agent_name)}
              </span>
              <span className="collab-toolbar-pill collab-micro">
                {channelDetail.member_count} 位成员
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
                协作方式说明
              </button>
              <span className="collab-meta truncate">
                {channelDetail.description || '讨论模式负责团队商讨，协作模式负责单条工作的推进，协作列表负责统一查看全部协作项。'}
              </span>
            </div>
            {autonomyGuideOpen ? renderAutonomyGuide() : null}

            <div className="collab-mode-switch mt-1">
              {([
                { key: 'discussion', label: '讨论模式', hint: '团队群聊', count: discussionAreaMessages.length },
                {
                  key: 'work',
                  label: '协作模式',
                  hint: activeThread ? collaborationSurfaceLabel(activeThread.surface) : '当前工作',
                  count: activeThread ? 1 : 0,
                },
                {
                  key: 'worklist',
                  label: '协作列表',
                  hint: workSurfaceFilter === 'all' ? '临时 + 正式' : collaborationSurfaceFilterLabel(workSurfaceFilter),
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
                        <span className="collab-kicker">讨论模式</span>
                        <span className="collab-header-title">团队群聊与商讨窗口</span>
                        <span className="collab-header-description">用于商量方向、同步结果、提醒成员，并决定哪些事情进入协作。</span>
                      </div>
                      <span className="collab-toolbar-pill collab-micro collab-header-count">{discussionAreaMessages.length} 条讨论</span>
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
                          <div className="collab-section-title">讨论区还没有内容</div>
                          <div className="collab-meta mt-3 max-w-[460px] leading-6">
                            先在这里发起团队讨论，确认方向、@相关成员，等事情清楚之后再转进协作模式。
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
                        <span className="collab-kicker shrink-0">协作模式</span>
                        <span className="collab-thread-title">
                          {activeThread ? currentThreadSummary : '进入一条具体协作项继续推进'}
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
                          <span className="collab-thread-description">
                          {activeThread
                            ? activeThread.surface === 'temporary'
                              ? '先围绕问题澄清、追问和试探方向，再决定是否升级成正式协作。'
                              : '围绕一件正式协作项持续推进、补充判断；长时间静默后会自动同步阶段进展到讨论区。'
                            : '协作模式一次只处理一条具体工作。'}
                        </span>
                        {activeThread ? (
                          <button
                            type="button"
                            onClick={() => setThreadRootExpanded((prev) => !prev)}
                            className="collab-thread-origin-link"
                          >
                            <ChevronRight className={`h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform ${threadRootExpanded ? 'rotate-90' : ''}`} />
                            <span className="collab-thread-origin-label">协作起点</span>
                            <span className="collab-thread-origin-text">{activeThread.author_name} · 点击查看最初发起内容</span>
                          </button>
                        ) : null}
                      </div>
                    </div>

                    {activeThread ? (
                      <div className="collab-thread-action-row">
                        {!['adopted', 'rejected'].includes(activeThread.display_status || '') ? (
                          <>
                            <Button type="button" size="sm" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestAdoptThreadConfirm(activeThread)}>标记采用</Button>
                            {activeThread.surface === 'temporary' ? (
                              <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestMarkThreadHighlightedConfirm(activeThread)}>升级为正式协作</Button>
                            ) : null}
                            {activeThread.display_status !== 'awaiting_confirmation' ? (
                              <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestAwaitingConfirmationConfirm(activeThread)}>等判断</Button>
                            ) : null}
                            <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestSyncThreadResultConfirm(activeThread)}>同步到讨论</Button>
                            <Button type="button" size="sm" variant="outline" className="h-7 rounded-full px-2.5 text-[11px]" onClick={() => requestArchiveThreadConfirm(activeThread)}>未采用</Button>
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
                          <div className="collab-section-title">还没有打开具体协作项</div>
                          <div className="collab-meta mt-3 max-w-[460px] leading-6">协作模式一次只处理一条具体工作。直接用上方三态切换到协作列表，或者打开最近的协作项继续推进。</div>
                          <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
                            {collaborationItems.length > 0 ? (
                              <Button type="button" onClick={() => void handleOpenThread(collaborationItems[0].message)}>
                                打开最近协作项
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
                          <span className="collab-kicker">协作列表</span>
                          <span className="collab-header-title">
                            {workSurfaceFilter === 'temporary'
                              ? '先聊明白的临时协作'
                              : workSurfaceFilter === 'issue'
                                ? '正在推进的正式协作'
                                : '查看全部协作项，再决定进入哪条工作'}
                          </span>
                          <span className="collab-header-description">
                            {workSurfaceFilter === 'temporary'
                              ? '这里集中查看 @Agent 打开的探讨线程和待澄清事项。'
                              : workSurfaceFilter === 'issue'
                                ? '这里集中查看已经进入正式推进的工作；静默一段时间后，系统会把阶段进展同步到讨论区。'
                                : '先按协作类型分区，再按状态筛选，选中后进入协作模式处理单条工作。'}
                          </span>
                        </div>
                        <span className="collab-toolbar-pill collab-micro collab-header-count">
                          {currentWorkOrUpdateItems.length} 条
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
                                    {item.message.source_kind === 'agent' ? 'Agent 发起' : '成员发起'}
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
                                    <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{item.message.reply_count} 条回复</span>
                                    {item.recentAgents.length > 0 ? (
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{item.recentAgents.slice(0, 2).join(' · ')}</span>
                                    ) : null}
                                    {item.documentCount > 0 ? (
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{item.documentCount} 资料</span>
                                    ) : null}
                                    {item.aiOutputCount > 0 ? (
                                      <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">{item.aiOutputCount} 个 AI 产出</span>
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
                <div className="collab-kicker">智能协作频道</div>
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
                    {channelDetail.member_count} 位成员
                  </span>
                  {channelDetail.is_processing ? (
                    <span className="collab-toolbar-pill collab-micro" data-active="true">
                      正在处理中
                    </span>
                  ) : null}
                </div>
                <p className="collab-meta mt-3 max-w-4xl leading-6">
                  {channelDetail.description || '讨论区负责商量、提醒、澄清与同步；协作项负责真正进入执行推进。频道资料和 AI 产出只在需要时展开查看。'}
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
                  信息面板
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
                      <div className="collab-kicker">讨论区</div>
                      <div className="collab-section-title mt-2">重点讨论与商量</div>
                      <div className="collab-meta mt-2 max-w-[560px] leading-5">
                        频道里的主要对话窗口。这里承接成员讨论、Agent 建议、阶段总结和结果同步，不直接替代协作执行。
                      </div>
                    </div>
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="collab-toolbar-pill collab-micro">
                        {discussionAreaMessages.length} 条消息
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
                        <div className="collab-section-title">讨论区还没有内容</div>
                        <div className="collab-meta mt-3 max-w-[420px] leading-6">
                          这里用于公开同步、提醒、@成员，以及由管理 Agent 发出的建议卡、总结卡和结果卡。
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
                            返回协作项
                          </button>
                          <div className="min-w-0 flex-1">
                            <div className="collab-kicker">协作项</div>
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
                            标记采用
                          </Button>
                          {activeThread.surface === 'temporary' ? (
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              className="h-8 rounded-[999px] px-3 text-[11px]"
                              onClick={() => requestMarkThreadHighlightedConfirm(activeThread)}
                            >
                              升级为正式协作
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
                              等你判断
                            </Button>
                          ) : null}
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-8 rounded-[999px] px-3 text-[11px]"
                            onClick={() => requestSyncThreadResultConfirm(activeThread)}
                          >
                            同步结果
                          </Button>
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-8 rounded-[999px] px-3 text-[11px]"
                            onClick={() => requestArchiveThreadConfirm(activeThread)}
                          >
                            标记未采用
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
                          <div className="collab-kicker">协作起点</div>
                          <div className="collab-meta mt-1 truncate">
                            {activeThread.author_name} · 点击查看这条协作项最初的发起内容
                          </div>
                        </div>
                      </button>
                      {threadRootExpanded ? <ThreadRootCard message={activeThread} /> : null}
                    </div>
                  ) : (
                    <div className="space-y-3">
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div className="min-w-0">
                          <div className="collab-kicker">协作项</div>
                          <div className="collab-section-title mt-2">
                            {workSurfaceFilter === 'temporary'
                              ? '先聊明白的临时协作'
                              : workSurfaceFilter === 'issue'
                                ? '正在推进的正式协作'
                                : '进入协作执行'}
                          </div>
                          <div className="collab-meta mt-2 max-w-[320px] leading-5">
                            {workSurfaceFilter === 'temporary'
                              ? '这里集中查看 @Agent 打开的探讨线程、补充上下文和待澄清事项。'
                              : workSurfaceFilter === 'issue'
                                ? '这里集中查看已经进入正式推进的工作；静默 1 小时后，阶段进展会同步到讨论区。'
                                : '先按协作类型分区，再按状态筛选，选中后进入协作模式处理单条工作。'}
                          </div>
                        </div>
                        <span className="collab-toolbar-pill collab-micro">
                          {currentWorkOrUpdateItems.length} 条
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
                        <div className="collab-section-title">还没有协作项</div>
                        <div className="collab-meta mt-3 max-w-[420px] leading-6">
                          先在讨论区把事情商量清楚，再把真正要推进的事项送进协作项。
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
                                    {item.message.reply_count} 条回复
                                  </span>
                                  {item.documentCount > 0 ? (
                                    <span className="collab-toolbar-pill collab-micro">
                                      {item.documentCount} 资料
                                    </span>
                                  ) : null}
                                  {item.aiOutputCount > 0 ? (
                                    <span className="collab-toolbar-pill collab-micro">
                                      {item.aiOutputCount} 个 AI 产出
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
                    <div className="collab-kicker">消息输入区</div>
                    <div className="collab-section-title mt-2">讨论先行，必要时进入协作</div>
                    <div className="collab-meta mt-2 max-w-[520px] leading-5">
                      默认先发到讨论区；只有当你明确要推进执行时，再直接创建协作项。
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
                    技能
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    className="h-9 rounded-[999px] px-3 text-[11px]"
                    onClick={() => setShowDocPicker(true)}
                  >
                    附件
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    className="h-9 rounded-[999px] px-3 text-[11px]"
                    onClick={() => fileInputRef.current?.click()}
                    disabled={uploadingDocument}
                  >
                    {uploadingDocument ? '上传中…' : '上传'}
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
                          title="移除文档"
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
                          title="移除能力"
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
                      ? `将围绕 @${rootMentionedAgentLabel} 打开一条临时协作会话，先探讨和澄清，不直接当成正式协作项。`
                      : '将发送到讨论区，适合同步、提醒、确认和 @成员；如果你要立即推进执行，可以直接创建协作项。'}
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
                        ? `和 @${rootMentionedAgentLabel} 先聊清楚问题、背景和下一步…`
                        : '发送讨论内容，和团队同步、提醒、澄清或 @成员…'
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
                        开始协作项
                      </Button>
                    ) : null}
                    <Button
                      onClick={() => void handleSend(undefined, rootStartsTemporaryCollaboration ? undefined : 'discussion')}
                      disabled={sending || (!composeText.trim() && selectedCapabilityRefs.length === 0)}
                      className="h-10 rounded-[999px] px-4 text-[12px]"
                    >
                      <Send className="mr-1 h-3.5 w-3.5" />
                      {rootStartsTemporaryCollaboration ? '开始临时协作' : '发送讨论'}
                    </Button>
                  </div>
                </div>
                {error ? <div className="pt-3 text-[12px] text-destructive">{error}</div> : null}
              </div>
            ) : (
              <div className="collab-composer-shell mt-4 px-5 py-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="collab-kicker">线程回复</span>
                    <span className="collab-meta">围绕当前协作项继续补充、修正和推进</span>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="relative min-w-[180px]">
                      {renderComposerTargetTrigger(
                        'thread',
                        'h-8 rounded-[999px] border border-[hsl(var(--ui-line-soft))/0.7] bg-[hsl(var(--ui-surface-panel-strong))/0.86] px-3 text-[11px] shadow-none',
                      )}
                    </div>
                    <Button type="button" variant="outline" className="h-8 rounded-[999px] px-3 text-[11px]" onClick={() => { setCapabilityDetailKey(null); setShowCapabilityPicker(true); }}>
                      技能
                    </Button>
                    <Button type="button" variant="outline" className="h-8 rounded-[999px] px-3 text-[11px]" onClick={() => setShowDocPicker(true)}>
                      附件
                    </Button>
                    <Button type="button" variant="outline" className="h-8 rounded-[999px] px-3 text-[11px]" onClick={() => fileInputRef.current?.click()} disabled={uploadingDocument}>
                      {uploadingDocument ? '上传中…' : '上传'}
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
                    placeholder="围绕这条协作项继续推进、补充判断或提出下一步…"
                  />
                  <div className="absolute bottom-3 right-3">
                    <Button
                      onClick={() => void handleSend(threadRootId)}
                      disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
                      className="h-9 rounded-[999px] px-4 text-[12px]"
                    >
                      <MessageSquareReply className="mr-1 h-3.5 w-3.5" />
                      回复协作项
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
              <div className="collab-kicker">频道列表</div>
              <div className="collab-section-title mt-2">智能协作频道</div>
              <div className="collab-meta mt-2 leading-5">从这里进入具体频道，在讨论区商量，在协作项里推进执行。</div>
            </div>
            <Button
              size="icon"
              variant="outline"
              className="h-10 w-10 shrink-0 rounded-full border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.86] shadow-none"
              onClick={() => setCreateOpen(true)}
              title="新建频道"
            >
              <Plus className="h-4 w-4" />
            </Button>
          </div>
          <div className="mt-3 flex items-center gap-2">
            <Input
              value={channelSearch}
              onChange={(event) => setChannelSearch(event.target.value)}
              placeholder="搜索频道、描述或最近消息"
              className="h-9 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none"
            />
            <Button
              variant="outline"
              className="h-9 shrink-0 rounded-[12px] border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--background))] px-3 text-[11px] shadow-none"
              onClick={() => setCreateOpen(true)}
            >
              新建频道
            </Button>
          </div>
          <div className="mt-4 flex items-center justify-between">
            <span className="collab-kicker">频道目录</span>
            <span className="collab-toolbar-pill collab-micro">
              {filteredChannels.length} 个结果
            </span>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-2.5 py-3">
          {loadingChannels ? (
            <div className="flex items-center justify-center p-6">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : channels.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground">还没有团队频道，先创建一个。</div>
          ) : filteredChannels.length === 0 ? (
            <div className="border-b border-dashed border-border/70 px-4 py-6 text-center">
              <div className="text-[13px] font-semibold text-foreground">没有匹配的频道</div>
              <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                试试换个关键词，或者直接创建一个新的协作频道。
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
                              title={channel.pinned ? '取消置顶' : '置顶频道'}
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
                              title={channel.muted ? '取消静音' : '静音频道'}
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
                        {channelDetail.member_count} 位成员
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
                        协作方式说明
                      </button>
                    </div>
                    {autonomyGuideOpen ? renderAutonomyGuide() : null}
                    {channelDetail.description ? (
                      <p className="mt-1.5 line-clamp-1 text-[11px] leading-5 text-muted-foreground">
                        {channelDetail.description}
                      </p>
                    ) : (
                      <p className="mt-1.5 text-[11px] leading-5 text-muted-foreground">
                        讨论在公共区发生，持续推进在协作项内完成。
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
                          资料
                        </Button>
                        <Button
                          variant={sidePanelMode === 'ai_outputs' ? 'default' : 'outline'}
                          size="sm"
                          onClick={() => setSidePanelMode((current) => current === 'ai_outputs' ? null : 'ai_outputs')}
                          className="h-8 rounded-[12px] px-2.5 text-[11px] shadow-none"
                        >
                          AI 产出
                        </Button>
                        <Button variant="ghost" size="sm" className="h-8 rounded-[12px] px-2.5 text-[11px]" onClick={() => setMembersOpen(true)}>
                          成员
                        </Button>
                        <Button variant="ghost" size="sm" className="h-8 rounded-[12px] px-2.5 text-[11px]" onClick={() => setSettingsOpen(true)}>
                          频道设置
                        </Button>
                      </>
                    ) : (
                      <Button
                        variant={sidePanelMode && sidePanelMode !== 'thread' ? 'default' : 'outline'}
                        size="sm"
                        onClick={() => setSidePanelMode((current) => (current && current !== 'thread' ? null : 'documents'))}
                        className="h-8 rounded-[10px] border-border/70 px-3 text-[11px] shadow-none"
                      >
                        信息面板
                      </Button>
                    )}
                  </div>
                </div>
              </div>

              <div className="border-b border-border/70 bg-background px-5 py-2.5">
                <div className="flex flex-wrap items-center gap-4">
                  <div className="flex items-center gap-5">
                    {([
                      { key: 'work', label: '协作项' },
                      { key: 'update', label: '讨论区' },
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
                      资料 {channelDocuments.length}
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
                      AI 产出 {channelAiOutputs.length}
                    </button>
                    {threadRootId && !isMobile ? (
                      <span className="rounded-full border border-border/70 bg-background px-2 py-0.5">
                        当前线程
                      </span>
                    ) : null}
                    <span className="ml-auto hidden text-[10px] text-muted-foreground/75 xl:inline">
                      默认仅读已附加资料
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
                          返回协作项列表
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
                              标记采用
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
                                    升级为正式协作
                                  </button>
                                ) : null}
                                {activeThread.display_status !== 'awaiting_confirmation' ? (
                                  <button
                                    type="button"
                                    onClick={() => requestAwaitingConfirmationConfirm(activeThread)}
                                    className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                  >
                                    等你判断
                                  </button>
                                ) : null}
                                <button
                                  type="button"
                                  onClick={() => requestSyncThreadResultConfirm(activeThread)}
                                  className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                >
                                  同步结果
                                </button>
                                <button
                                  type="button"
                                  onClick={() => requestArchiveThreadConfirm(activeThread)}
                                  className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                                >
                                  标记未采用
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
                          协作起点 · {activeThread.author_name} · 点击查看起点详情
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
                          技能
                        </Button>
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => setShowDocPicker(true)}
                          className="h-8 rounded-[10px] border-border/60 bg-background px-2.5 text-[11px] shadow-none"
                        >
                          附件
                        </Button>
                        <Button
                          type="button"
                          variant="outline"
                          onClick={() => fileInputRef.current?.click()}
                          disabled={uploadingDocument}
                          className="h-8 rounded-[10px] border-border/60 bg-background px-2.5 text-[11px] shadow-none"
                        >
                          {uploadingDocument ? '上传中…' : '上传'}
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
                          placeholder="围绕这条线程继续推进…"
                        />
                        <div className="absolute bottom-2 right-2">
                          <Button
                            onClick={() => void handleSend(threadRootId)}
                            disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
                            className="h-8 rounded-[12px] px-3 text-[12px] shadow-none"
                          >
                            <MessageSquareReply className="mr-1 h-3.5 w-3.5" />
                            回复
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
                            <div className="text-[12px] font-medium text-foreground">讨论区还没有内容</div>
                            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                              这里用于团队交流、提醒、同步和 @成员，不默认进入协作过程。
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
                          <div className="text-[13px] font-semibold text-foreground">协作项</div>
                          {workSurfaceFilter !== 'all' || workStatusFilter !== 'all' ? (
                            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                              {`当前筛选：${workSurfaceFilter === 'all' ? '全部协作' : collaborationSurfaceFilterLabel(workSurfaceFilter)}${
                                workStatusFilter === 'all' ? '' : ` · ${collaborationStatusFilterLabel(workStatusFilter)}`
                              }`}
                            </div>
                          ) : null}
                          </div>
                          <div className="text-[11px] text-muted-foreground">
                            {currentWorkOrUpdateItems.length} 条
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
                                        {item.message.source_kind === 'agent' ? 'Agent 发起' : '成员发起'}
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
                                        {item.message.reply_count} 条回复
                                      </span>
                                      {item.recentAgents.length > 0 ? (
                                        <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                          {item.recentAgents.slice(0, 2).join(' · ')}
                                        </span>
                                      ) : null}
                                      {item.documentCount > 0 ? (
                                        <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                          {item.documentCount} 资料
                                        </span>
                                      ) : null}
                                      {item.aiOutputCount > 0 ? (
                                        <span className="rounded-full bg-muted/45 px-2 py-0.5 text-muted-foreground">
                                          {item.aiOutputCount} 个 AI 产出
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
                              工具
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
                                技能
                              </Button>
                              <Button
                                type="button"
                                variant="outline"
                                onClick={() => setShowDocPicker(true)}
                                className="h-8 rounded-[10px] border-border/70 bg-background px-2.5 text-[11px] shadow-none"
                              >
                                附件
                              </Button>
                              <Button
                                type="button"
                                variant="outline"
                                onClick={() => fileInputRef.current?.click()}
                                disabled={uploadingDocument}
                                className="h-8 rounded-[10px] border-border/70 bg-background px-2.5 text-[11px] shadow-none"
                              >
                                {uploadingDocument ? '上传中…' : '上传'}
                              </Button>
                            </>
                          )}
                        </div>
                        {!isMobile && (attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                          <div className="flex flex-wrap items-center justify-end gap-1.5">
                            {attachedDocs.length > 0 ? (
                              <span className="inline-flex items-center rounded-full bg-background/90 px-2.5 py-1 text-[10.5px] text-muted-foreground">
                                <Paperclip className="mr-1 h-3 w-3" />
                                {attachedDocs.length} 个附件
                              </span>
                            ) : null}
                            {selectedCapabilities.length > 0 ? (
                              <span className="inline-flex items-center rounded-full bg-primary/[0.08] px-2.5 py-1 text-[10.5px] text-primary">
                                <Sparkles className="mr-1 h-3 w-3" />
                                {selectedCapabilities.length} 个技能
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
                                title="移除文档"
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
                                title="移除能力"
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
                            ? `将围绕 @${rootMentionedAgentLabel} 打开一条临时协作会话，先探讨和澄清。`
                            : rootComposerIntent === 'work'
                              ? '将开始一个协作项，AI 和团队成员会围绕这件事继续推进'
                              : '发送到讨论区，适合人与人交流、提醒、同步和 @成员'}
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
                              ? `和 @${rootMentionedAgentLabel} 先聊清楚问题、背景和下一步…`
                              : rootComposerIntent === 'work'
                                ? '开始一项协作，说明要推进什么…'
                                : '发送讨论内容，和团队成员同步、提醒或 @成员…'
                          }
                        />
                        <div className="absolute bottom-2 right-2 flex items-center gap-2">
                          {isMobile && (attachedDocs.length > 0 || selectedCapabilities.length > 0) ? (
                            <span className="hidden rounded-full bg-muted/40 px-2 py-1 text-[10.5px] text-muted-foreground sm:inline-flex">
                              {attachedDocs.length > 0 ? `${attachedDocs.length} 附件` : ''}
                              {attachedDocs.length > 0 && selectedCapabilities.length > 0 ? ' · ' : ''}
                              {selectedCapabilities.length > 0 ? `${selectedCapabilities.length} 技能` : ''}
                            </span>
                          ) : null}
                          <Button
                            onClick={() => void handleSend()}
                            disabled={sending || (!composeText.trim() && selectedCapabilityRefs.length === 0)}
                            className="h-8 rounded-[10px] px-3 text-[12px] shadow-none"
                          >
                            <Send className="mr-1 h-3.5 w-3.5" />
                            {rootStartsTemporaryCollaboration
                              ? '开始临时协作'
                              : rootComposerIntent === 'work'
                                ? '开始协作'
                                : '发送到讨论区'}
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
                <div className="collab-section-title">选择一个团队频道进入工作台</div>
                <div className="collab-meta mt-3 max-w-[420px] leading-6">
                  在这里打开讨论流、协作项、频道资料和 AI 产出，开始一条完整的协作路径。
                </div>
              </div>
            </div>
          )}
        </div>
        {renderDesktopInspector()}
      </div>

      <BottomSheetPanel open={isMobile && sidePanelMode === 'thread' && !!threadRootId} onOpenChange={(open) => { if (!open) { closeThreadPanel(); } }} title="协作项" description="围绕这件事继续推进" fullHeight>
        <div className="space-y-4">
          {threadRootMessage ? (
            <>
              <div className="rounded-[18px] bg-background px-3 py-3 shadow-[inset_0_0_0_1px_rgba(15,23,42,0.06)]">
                <div className="flex items-center gap-2">
                  <div className="text-[10px] font-medium uppercase tracking-[0.08em] text-muted-foreground">
                    协作项
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
                {!['adopted', 'rejected'].includes(threadRootMessage.display_status || '') ? (
                  <div className="mt-3 flex flex-wrap items-center gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="h-8 rounded-[12px] px-3 text-[11px] shadow-none"
                      onClick={() => requestAdoptThreadConfirm(threadRootMessage)}
                    >
                      标记采用
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
                            升级为正式协作
                          </button>
                        ) : null}
                        {threadRootMessage.display_status !== 'awaiting_confirmation' ? (
                          <button
                            type="button"
                            onClick={() => requestAwaitingConfirmationConfirm(threadRootMessage)}
                            className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                          >
                            等你判断
                          </button>
                        ) : null}
                        <button
                          type="button"
                          onClick={() => requestSyncThreadResultConfirm(threadRootMessage)}
                          className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                        >
                          同步结果
                        </button>
                        <button
                          type="button"
                          onClick={() => requestArchiveThreadConfirm(threadRootMessage)}
                          className="flex w-full items-center rounded-[8px] px-2.5 py-2 text-left text-[11px] text-foreground transition-colors hover:bg-accent/30"
                        >
                          标记未采用
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
                  协作起点 · {threadRootMessage.author_name} · 点击查看起点详情
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
                工具
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
                placeholder="围绕这条消息继续推进…"
              />
              <div className="absolute bottom-2 right-2">
                <Button
                  onClick={() => void handleSend(threadRootId)}
                  disabled={sending || (!threadComposeText.trim() && selectedCapabilityRefs.length === 0)}
                  className="h-8 rounded-[12px] px-3 text-[12px] shadow-none"
                >
                  <MessageSquareReply className="mr-1 h-3.5 w-3.5" />回复
                </Button>
              </div>
            </div>
          </div>
        </div>
      </BottomSheetPanel>

      <BottomSheetPanel
        open={isMobile && (sidePanelMode === 'documents' || sidePanelMode === 'ai_outputs')}
        onOpenChange={(open) => {
          if (!open) {
            setSidePanelMode(null);
          }
        }}
        title={sidePanelMode === 'ai_outputs' ? 'AI 产出' : '频道资料'}
        description={sidePanelMode === 'ai_outputs' ? '查看当前频道的 AI 草稿与结果' : '查看当前频道的资料与文档目录'}
        fullHeight
      >
        {sidePanelMode === 'ai_outputs' ? renderAiOutputsInspector() : renderDocumentsInspector()}
      </BottomSheetPanel>

      <BottomSheetPanel
        open={composerToolsOpen}
        onOpenChange={setComposerToolsOpen}
        title="频道工具"
        description="把技能、文档和上传资料挂到当前频道消息里。"
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
              <div className="text-[13px] font-medium text-foreground">技能</div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                选择当前 Agent 真正可调用的技能和扩展。
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
              <div className="text-[13px] font-medium text-foreground">附件</div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                把团队文档附加到当前频道消息或当前线程；只有附加后的文档才会进入本轮上下文。
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
                {uploadingDocument ? '上传中…' : '上传文件'}
              </div>
              <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                直接上传到本频道的文档目录，并附加到当前消息；不会因为上传就自动全量读取频道资料。
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
        title="发布到团队文档"
        description="把这份 AI 产出升级成团队正式文档，并选择展示名称与目标目录。"
        fullHeight={!isMobile}
      >
        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-[12px] text-muted-foreground">展示名称</label>
            <Input
              value={publishName}
              onChange={(event) => setPublishName(event.target.value)}
              placeholder="文档名称"
            />
          </div>
          <div>
            <label className="mb-1 block text-[12px] text-muted-foreground">目标目录</label>
            <Select value={publishFolderPath} onValueChange={setPublishFolderPath}>
              <SelectTrigger>
                <SelectValue placeholder="选择目录" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="/">根目录</SelectItem>
                {flattenFolders(folderTree).map((folder) => (
                  <SelectItem key={folder.path} value={folder.path}>
                    {folder.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {foldersLoading ? (
              <div className="mt-2 text-[11px] text-muted-foreground">正在读取团队文档目录…</div>
            ) : null}
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={closePublishDialog}>
              取消
            </Button>
            <Button
              onClick={() => void confirmPublishAiOutput()}
              disabled={!publishTargetDoc || publishingDocId === publishTargetDoc.id}
            >
              {publishTargetDoc && publishingDocId === publishTargetDoc.id ? '发布中…' : '确认发布'}
            </Button>
          </div>
        </div>
      </BottomSheetPanel>

      <BottomSheetPanel open={createOpen} onOpenChange={setCreateOpen} title="创建团队频道" description="公开频道对团队成员可见，私密频道只对指定成员可见。" fullHeight={!isMobile}>
        <div className="space-y-4">
          <Input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} placeholder="频道名称" />
          <Textarea value={form.description} onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))} placeholder="频道说明" className="min-h-[88px]" />
          <Select value={form.visibility} onValueChange={(value) => setForm((prev) => ({ ...prev, visibility: value as ChatChannelVisibility }))}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="team_public">公开频道</SelectItem>
              <SelectItem value="team_private">私密频道</SelectItem>
            </SelectContent>
          </Select>
          <Select value={form.defaultAgentId} onValueChange={(value) => setForm((prev) => ({ ...prev, defaultAgentId: value }))}>
            <SelectTrigger><SelectValue placeholder="默认 Agent" /></SelectTrigger>
            <SelectContent>
              {visibleAgents.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          {form.visibility === 'team_private' ? (
            <div className="space-y-2">
              <div className="text-[12px] font-medium text-foreground">初始成员</div>
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
              <Plus className="mr-1 h-4 w-4" />创建频道
            </Button>
          </div>
        </div>
      </BottomSheetPanel>

      <BottomSheetPanel open={settingsOpen} onOpenChange={setSettingsOpen} title="频道设置" description="维护频道名称、描述、可见范围和默认 Agent。" fullHeight={!isMobile}>
        <div className="space-y-4">
          <Input value={form.name} onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))} placeholder="频道名称" />
          <Textarea value={form.description} onChange={(event) => setForm((prev) => ({ ...prev, description: event.target.value }))} placeholder="频道说明" className="min-h-[88px]" />
          <Select value={form.visibility} onValueChange={(value) => setForm((prev) => ({ ...prev, visibility: value as ChatChannelVisibility }))}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="team_public">公开频道</SelectItem>
              <SelectItem value="team_private">私密频道</SelectItem>
            </SelectContent>
          </Select>
          <Select value={form.defaultAgentId} onValueChange={(value) => setForm((prev) => ({ ...prev, defaultAgentId: value }))}>
            <SelectTrigger><SelectValue placeholder="默认 Agent" /></SelectTrigger>
            <SelectContent>
              {visibleAgents.map((agent) => (
                <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="space-y-1">
            <div className="text-[12px] text-muted-foreground">管理 Agent 主动度</div>
            <Select
              value={form.agentAutonomyMode}
              onValueChange={(value) =>
                setForm((prev) => ({ ...prev, agentAutonomyMode: value as ChatChannelAgentAutonomyMode }))
              }
            >
              <SelectTrigger><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="standard">标准模式</SelectItem>
                <SelectItem value="proactive">主动推进模式</SelectItem>
                <SelectItem value="agent_lead">Agent 主导模式</SelectItem>
              </SelectContent>
            </Select>
            <div className="text-[11px] leading-5 text-muted-foreground">
              {getChannelAutonomyMeta(form.agentAutonomyMode).summary}
            </div>
          </div>
          <div className="space-y-3 rounded-[16px] border border-border/70 bg-muted/[0.04] px-4 py-3">
            <div>
              <div className="text-[13px] font-medium text-foreground">频道记忆</div>
              <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
                管理 Agent 会长期参考这里的信息来判断建议、总结、提醒和推进方向。
              </div>
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">频道目标</div>
              <Textarea
                value={form.channelGoal}
                onChange={(event) => setForm((prev) => ({ ...prev, channelGoal: event.target.value }))}
                placeholder="这个频道主要围绕什么事情建立？"
                className="min-h-[72px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">参与人</div>
              <Textarea
                value={form.participantNotes}
                onChange={(event) => setForm((prev) => ({ ...prev, participantNotes: event.target.value }))}
                placeholder="主要谁会参与协作？谁是关键判断人？"
                className="min-h-[72px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">产出物</div>
              <Textarea
                value={form.expectedOutputs}
                onChange={(event) => setForm((prev) => ({ ...prev, expectedOutputs: event.target.value }))}
                placeholder="最终希望形成什么结果、文档、方案或交付物？"
                className="min-h-[72px]"
              />
            </div>
            <div className="space-y-1">
              <div className="text-[12px] text-muted-foreground">协作方式</div>
              <Input
                value={form.collaborationStyle}
                onChange={(event) => setForm((prev) => ({ ...prev, collaborationStyle: event.target.value }))}
                placeholder="例如：偏讨论 / 偏方案 / 偏执行 / 偏评审 / 混合"
              />
            </div>
            {channelDetail?.orchestrator_state?.last_heartbeat_at ? (
              <div className="rounded-[12px] bg-background/80 px-3 py-2 text-[11px] text-muted-foreground">
                最近心跳：{formatDateTime(channelDetail.orchestrator_state.last_heartbeat_at)}
                {channelDetail.orchestrator_state.last_heartbeat_reason
                  ? ` · ${channelDetail.orchestrator_state.last_heartbeat_reason}`
                  : ''}
              </div>
            ) : null}
          </div>
          <div className="flex justify-end">
            <Button onClick={() => void handleSaveChannel()}>
              <Save className="mr-1 h-4 w-4" />保存频道设置
            </Button>
          </div>
          <div className="rounded-[16px] border border-destructive/20 bg-destructive/[0.04] px-4 py-3">
            <div className="text-[13px] font-medium text-foreground">删除频道</div>
            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
              可以选择只删除频道，保留所有文档；或者彻底删除频道和相关文档。
            </div>
            <div className="mt-3 flex justify-end">
              <Button variant="destructive" onClick={() => setDeleteDialogOpen(true)}>
                删除频道
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
        title={pendingCollaborationAction?.title || '确认操作'}
        description={pendingCollaborationAction?.description}
        confirmText={pendingCollaborationAction?.confirmText}
        variant={pendingCollaborationAction?.variant || 'default'}
        onConfirm={() => void handleConfirmCollaborationAction()}
        loading={confirmingCollaborationAction}
        cancelText="取消"
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
        title="删除频道"
        description="选择删除方式。这个动作无法撤销。"
        confirmText={deleteMode === 'full_delete' ? '确认彻底删除' : '确认删除并保留文档'}
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
            <div className="text-[13px] font-medium text-foreground">保留所有文档删除</div>
            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
              删除频道、消息、成员和线程，但保留频道资料与 AI 产出文档。
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
            <div className="text-[13px] font-medium text-foreground">彻底删除</div>
            <div className="mt-1 text-[11px] leading-5 text-muted-foreground">
              删除频道、消息、成员、线程，以及当前频道目录下的资料和该频道来源的 AI 产出。
            </div>
          </button>
        </div>
      </ConfirmDialog>

      <BottomSheetPanel open={membersOpen} onOpenChange={setMembersOpen} title="频道成员" description="公开频道对团队成员可见；私密频道需要显式维护成员。" fullHeight={!isMobile}>
        <div className="space-y-4">
          <div className="flex items-center gap-2">
            <Select value={newMemberId} onValueChange={setNewMemberId}>
              <SelectTrigger><SelectValue placeholder="选择成员" /></SelectTrigger>
              <SelectContent>
                {memberOptions.map((member) => (
                  <SelectItem key={member.id} value={member.userId}>{member.displayName}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select value={newMemberRole} onValueChange={(value) => setNewMemberRole(value as 'member' | 'manager')}>
              <SelectTrigger className="w-[132px]"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="member">成员</SelectItem>
                <SelectItem value="manager">管理者</SelectItem>
              </SelectContent>
            </Select>
            <Button variant="outline" onClick={() => void handleAddMember()} disabled={!newMemberId}>
              添加
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
                  移除
                </Button>
              </div>
            ))}
          </div>
        </div>
      </BottomSheetPanel>
    </div>
  );
}


