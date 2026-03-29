import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  Bot,
  Hash,
  Loader2,
  Lock,
  MessageSquareReply,
  Plus,
  Save,
  Send,
  Settings2,
  Users,
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
import { useIsMobile } from '../../hooks/useMediaQuery';
import { chatApi, type ChatChannelDetail, type ChatChannelMember, type ChatChannelMessage, type ChatChannelSummary, type ChatChannelVisibility } from '../../api/chat';
import type { TeamAgent } from '../../api/agent';
import { ChatMessageBubble, type ToolCallInfo } from '../chat/ChatMessageBubble';
import { fetchVisibleChatAgents } from '../chat/visibleChatAgents';
import { apiClient } from '../../api/client';
import type { TeamMember } from '../../api/types';
import { useAuth } from '../../contexts/AuthContext';

type ChannelRenderMessage = ChatChannelMessage & {
  thinking?: string;
  toolCalls?: ToolCallInfo[];
  turn?: { current: number; max: number };
  compaction?: { strategy: string; before: number; after: number };
  isStreaming?: boolean;
};

interface TeamChannelsPanelProps {
  teamId: string;
}

interface ChannelFormState {
  name: string;
  description: string;
  visibility: ChatChannelVisibility;
  defaultAgentId: string;
  memberUserIds: string[];
}

function emptyForm(defaultAgentId = ''): ChannelFormState {
  return {
    name: '',
    description: '',
    visibility: 'team_public',
    defaultAgentId,
    memberUserIds: [],
  };
}

function channelVisibilityLabel(visibility: ChatChannelVisibility) {
  return visibility === 'team_private' ? '私密频道' : '公开频道';
}

function formatDateTime(value?: string | null) {
  if (!value) return '';
  return new Date(value).toLocaleString();
}

function fromChannelMessage(message: ChatChannelMessage): ChannelRenderMessage {
  return { ...message };
}

function buildOptimisticUserMessage(
  channelId: string,
  user: TeamMember | null,
  content: string,
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
    content_text: content,
    content_blocks: [],
    metadata: {},
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

function ChannelUserBubble({
  message,
  isOwn,
  onReply,
  onOpenThread,
}: {
  message: ChannelRenderMessage;
  isOwn: boolean;
  onReply: (message: ChannelRenderMessage) => void;
  onOpenThread: (message: ChannelRenderMessage) => void;
}) {
  const timestamp = new Date(message.created_at);
  if (isOwn) {
    return (
      <div>
        <ChatMessageBubble
          role="user"
          content={message.content_text}
          timestamp={timestamp}
          userName={message.author_name}
        />
        <div className="mt-1 flex justify-end gap-2 px-2 text-[11px] text-muted-foreground">
          <button type="button" onClick={() => onReply(message)} className="hover:text-foreground">
            回复
          </button>
          <button type="button" onClick={() => onOpenThread(message)} className="hover:text-foreground">
            线程{message.reply_count > 0 ? ` (${message.reply_count})` : ''}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="mb-4">
      <div className="mb-1 px-1 text-[11px] text-muted-foreground">{message.author_name}</div>
      <div className="max-w-[82%] rounded-[18px] border border-border/60 bg-card px-3 py-2">
        <div className="whitespace-pre-wrap break-words text-[13px] leading-6 text-foreground">
          {message.content_text}
        </div>
        <div className="mt-2 text-[11px] text-muted-foreground">{formatDateTime(message.created_at)}</div>
      </div>
      <div className="mt-1 flex items-center gap-2 px-1 text-[11px] text-muted-foreground">
        <button type="button" onClick={() => onReply(message)} className="hover:text-foreground">
          回复
        </button>
        <button type="button" onClick={() => onOpenThread(message)} className="hover:text-foreground">
          线程{message.reply_count > 0 ? ` (${message.reply_count})` : ''}
        </button>
      </div>
    </div>
  );
}

function ChannelAssistantBubble({
  message,
  onReply,
  onOpenThread,
}: {
  message: ChannelRenderMessage;
  onReply: (message: ChannelRenderMessage) => void;
  onOpenThread: (message: ChannelRenderMessage) => void;
}) {
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
        agentName={message.author_name}
      />
      {!message.isStreaming && (
        <div className="mt-1 flex items-center gap-2 px-2 text-[11px] text-muted-foreground">
          <button type="button" onClick={() => onReply(message)} className="hover:text-foreground">
            回复
          </button>
          <button type="button" onClick={() => onOpenThread(message)} className="hover:text-foreground">
            线程{message.reply_count > 0 ? ` (${message.reply_count})` : ''}
          </button>
        </div>
      )}
    </div>
  );
}

function ChannelSystemBubble({ message }: { message: ChannelRenderMessage }) {
  return (
    <div className="flex justify-center">
      <div className="rounded-full bg-muted px-3 py-1 text-[11px] text-muted-foreground">
        {message.content_text}
      </div>
    </div>
  );
}

export function TeamChannelsPanel({ teamId }: TeamChannelsPanelProps) {
  useTranslation();
  const navigate = useNavigate();
  const { user } = useAuth();
  const isMobile = useIsMobile();
  const [channels, setChannels] = useState<ChatChannelSummary[]>([]);
  const [selectedChannelId, setSelectedChannelId] = useState<string | null>(null);
  const [channelDetail, setChannelDetail] = useState<ChatChannelDetail | null>(null);
  const [messages, setMessages] = useState<ChannelRenderMessage[]>([]);
  const [threadRootId, setThreadRootId] = useState<string | null>(null);
  const [threadMessages, setThreadMessages] = useState<ChannelRenderMessage[]>([]);
  const [threadRootMessage, setThreadRootMessage] = useState<ChannelRenderMessage | null>(null);
  const [members, setMembers] = useState<ChatChannelMember[]>([]);
  const [teamMembers, setTeamMembers] = useState<TeamMember[]>([]);
  const [visibleAgents, setVisibleAgents] = useState<TeamAgent[]>([]);
  const [loadingChannels, setLoadingChannels] = useState(true);
  const [loadingMessages, setLoadingMessages] = useState(false);
  const [sending, setSending] = useState(false);
  const [composeText, setComposeText] = useState('');
  const [threadComposeText, setThreadComposeText] = useState('');
  const [selectedAgentId, setSelectedAgentId] = useState('');
  const [createOpen, setCreateOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [membersOpen, setMembersOpen] = useState(false);
  const [form, setForm] = useState<ChannelFormState>(emptyForm());
  const [newMemberId, setNewMemberId] = useState('');
  const [newMemberRole, setNewMemberRole] = useState<'member' | 'manager'>('member');
  const [error, setError] = useState<string | null>(null);
  const streamRef = useRef<EventSource | null>(null);
  const streamThreadRootRef = useRef<string | null>(null);
  const lastEventIdRef = useRef<number | null>(null);

  const closeStream = useCallback(() => {
    streamRef.current?.close();
    streamRef.current = null;
    lastEventIdRef.current = null;
    streamThreadRootRef.current = null;
  }, []);

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
      setSelectedChannelId((current) => current || channelList[0]?.channel_id || null);
      setError(null);
    } catch (loadError) {
      console.error('Failed to load team channels:', loadError);
      setError('当前无法读取团队频道，请稍后再试。');
    } finally {
      setLoadingChannels(false);
    }
  }, [teamId]);

  const loadChannel = useCallback(async (channelId: string) => {
    setLoadingMessages(true);
    try {
      const [detail, rootMessages, memberList] = await Promise.all([
        chatApi.getChannel(channelId),
        chatApi.listChannelMessages(channelId),
        chatApi.listChannelMembers(channelId),
      ]);
      setChannelDetail(detail);
      setChannels((prev) =>
        prev.map((item) => (item.channel_id === detail.channel_id ? detail : item)),
      );
      setMembers(memberList);
      setMessages(rootMessages.map(fromChannelMessage));
      setSelectedAgentId(detail.default_agent_id);
      setError(null);
      await chatApi.markChannelRead(channelId);
    } catch (loadError) {
      console.error('Failed to load channel:', loadError);
      setError('当前无法读取频道详情，请稍后再试。');
    } finally {
      setLoadingMessages(false);
    }
  }, []);

  const loadThread = useCallback(async (channelId: string, rootId: string) => {
    try {
      const thread = await chatApi.getChannelThread(channelId, rootId);
      setThreadRootMessage(fromChannelMessage(thread.root_message));
      setThreadMessages(thread.messages.map(fromChannelMessage));
      setThreadRootId(rootId);
    } catch (loadError) {
      console.error('Failed to load channel thread:', loadError);
    }
  }, []);

  useEffect(() => {
    void loadChannels();
    return () => {
      closeStream();
    };
  }, [closeStream, loadChannels]);

  useEffect(() => {
    if (!selectedChannelId) {
      setChannelDetail(null);
      setMessages([]);
      return;
    }
    void loadChannel(selectedChannelId);
  }, [loadChannel, selectedChannelId]);

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
    (channelId: string, threadTarget?: string | null) => {
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

      es.addEventListener('text', (event) => {
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

      es.addEventListener('done', async () => {
        closeStream();
        setSending(false);
        await loadChannel(channelId);
        if (targetThreadRoot) {
          await loadThread(channelId, targetThreadRoot);
        }
      });

      es.onerror = () => {
        closeStream();
        setSending(false);
      };
    },
    [closeStream, loadChannel, loadThread, updateStreamingMessage],
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
      void loadThread(message.channel_id, message.message_id);
    },
    [loadThread],
  );

  const handleReply = useCallback(
    (message: ChannelRenderMessage) => {
      if (!message.thread_root_id) {
        void loadThread(message.channel_id, message.message_id);
      } else {
        void loadThread(message.channel_id, message.thread_root_id);
      }
    },
    [loadThread],
  );

  const handleSend = useCallback(
    async (threadTarget?: string | null) => {
      if (!channelDetail) return;
      const content = (threadTarget ? threadComposeText : composeText).trim();
      if (!content || sending) return;
      const currentMember =
        teamMembers.find((item) => item.userId === user?.id) || null;
      const selectedAgent =
        visibleAgents.find((agent) => agent.id === selectedAgentId) ||
        visibleAgents.find((agent) => agent.id === channelDetail.default_agent_id) ||
        null;
      const optimisticUser = buildOptimisticUserMessage(
        channelDetail.channel_id,
        currentMember,
        content,
        threadTarget,
        threadTarget || null,
      );
      const optimisticAssistant = buildStreamingAssistantMessage(
        channelDetail.channel_id,
        selectedAgent?.name || channelDetail.default_agent_name,
        threadTarget,
        optimisticUser.message_id,
      );
      if (threadTarget) {
        setThreadMessages((prev) => [...prev, optimisticUser, optimisticAssistant]);
        setThreadComposeText('');
      } else {
        setMessages((prev) => [...prev, optimisticUser, optimisticAssistant]);
        setComposeText('');
      }
      setSending(true);
      try {
        if (threadTarget) {
          await chatApi.sendChannelThreadMessage(channelDetail.channel_id, threadTarget, {
            content,
            agent_id: selectedAgentId || channelDetail.default_agent_id,
            parent_message_id: optimisticUser.message_id,
            mentions: [],
          });
        } else {
          await chatApi.sendChannelMessage(channelDetail.channel_id, {
            content,
            agent_id: selectedAgentId || channelDetail.default_agent_id,
            parent_message_id: null,
            mentions: [],
          });
        }
        openStream(channelDetail.channel_id, threadTarget);
      } catch (sendError) {
        console.error('Failed to send channel message:', sendError);
        setSending(false);
        setError('发送频道消息失败，请稍后再试。');
        if (threadTarget) {
          await loadThread(channelDetail.channel_id, threadTarget);
        } else {
          await loadChannel(channelDetail.channel_id);
        }
      }
    },
    [
      channelDetail,
      composeText,
      loadChannel,
      loadThread,
      members,
      openStream,
      selectedAgentId,
      sending,
      teamMembers,
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

  const renderMessage = useCallback(
    (message: ChannelRenderMessage) => {
      if (message.author_type === 'system') {
        return <ChannelSystemBubble key={message.message_id} message={message} />;
      }
      if (message.author_type === 'agent') {
        return (
          <ChannelAssistantBubble
            key={message.message_id}
            message={message}
            onReply={handleReply}
            onOpenThread={handleOpenThread}
          />
        );
      }
      return (
        <ChannelUserBubble
          key={message.message_id}
          message={message}
          isOwn={message.author_user_id === user?.id}
          onReply={handleReply}
          onOpenThread={handleOpenThread}
        />
      );
    },
    [handleOpenThread, handleReply, user?.id],
  );

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1 overflow-hidden bg-background">
      <div className="flex w-full shrink-0 flex-col border-r border-border/60 bg-muted/10 md:w-[min(32vw,280px)] lg:w-[min(26vw,320px)]">
        <div className="flex items-center justify-between border-b border-border/60 px-3 py-3">
          <div>
            <div className="text-[11px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
              团队频道
            </div>
            <div className="text-[13px] text-muted-foreground">共享历史与线程协作</div>
          </div>
          <Button size="icon" variant="ghost" onClick={() => setCreateOpen(true)}>
            <Plus className="h-4 w-4" />
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          {loadingChannels ? (
            <div className="flex items-center justify-center p-6">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : channels.length === 0 ? (
            <div className="p-4 text-sm text-muted-foreground">还没有团队频道，先创建一个。</div>
          ) : (
            channels.map((channel) => (
              <button
                key={channel.channel_id}
                type="button"
                onClick={() => setSelectedChannelId(channel.channel_id)}
                className={`w-full border-l-2 px-3 py-3 text-left transition-colors ${
                  channel.channel_id === selectedChannelId
                    ? 'border-l-primary bg-accent/40'
                    : 'border-l-transparent hover:bg-accent/20'
                }`}
              >
                <div className="flex items-center gap-2">
                  {channel.visibility === 'team_private' ? (
                    <Lock className="h-3.5 w-3.5 text-muted-foreground" />
                  ) : (
                    <Hash className="h-3.5 w-3.5 text-muted-foreground" />
                  )}
                  <div className="min-w-0 flex-1 truncate text-[13px] font-medium text-foreground">
                    {channel.name}
                  </div>
                  {channel.unread_count > 0 ? (
                    <span className="rounded-full bg-primary px-1.5 py-0.5 text-[10px] text-primary-foreground">
                      {channel.unread_count}
                    </span>
                  ) : null}
                </div>
                {channel.description ? (
                  <div className="mt-1 line-clamp-2 text-[11px] leading-4 text-muted-foreground">
                    {channel.description}
                  </div>
                ) : null}
                <div className="mt-2 flex items-center gap-2 text-[10px] text-muted-foreground">
                  <span>{channelVisibilityLabel(channel.visibility)}</span>
                  <span>·</span>
                  <span>{channel.default_agent_name}</span>
                </div>
              </button>
            ))
          )}
        </div>
      </div>

      <div className="flex min-w-0 flex-1 overflow-hidden">
        <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
          {channelDetail ? (
            <>
              <div className="border-b border-border/60 px-4 py-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <div className="text-[16px] font-semibold text-foreground">{channelDetail.name}</div>
                      <span className="rounded-full border border-border/70 px-2 py-0.5 text-[10px] text-muted-foreground">
                        {channelVisibilityLabel(channelDetail.visibility)}
                      </span>
                    </div>
                    {channelDetail.description ? (
                      <p className="mt-1 text-[12px] leading-5 text-muted-foreground">{channelDetail.description}</p>
                    ) : null}
                    <div className="mt-2 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                      <span className="inline-flex items-center gap-1"><Bot className="h-3.5 w-3.5" />默认 Agent · {channelDetail.default_agent_name}</span>
                      <span className="inline-flex items-center gap-1"><Users className="h-3.5 w-3.5" />{channelDetail.member_count} 位成员</span>
                      {channelDetail.document_folder_path ? (
                        <button
                          type="button"
                          onClick={() => openChannelDocuments(channelDetail.document_folder_path)}
                          className="inline-flex items-center gap-1 text-primary hover:underline"
                        >
                          文档文件夹 · {channelDetail.document_folder_path}
                        </button>
                      ) : null}
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <Button variant="outline" size="sm" onClick={() => setMembersOpen(true)}>
                      <Users className="mr-1 h-3.5 w-3.5" />成员
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => setSettingsOpen(true)}>
                      <Settings2 className="mr-1 h-3.5 w-3.5" />频道设置
                    </Button>
                  </div>
                </div>
              </div>

              <div className="flex-1 overflow-y-auto px-4 py-4">
                {loadingMessages ? (
                  <div className="flex items-center justify-center py-10">
                    <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
                  </div>
                ) : (
                  <div className="space-y-4">{messages.map(renderMessage)}</div>
                )}
              </div>

              <div className="border-t border-border/60 px-4 py-3">
                <div className="flex items-end gap-2">
                  <div className="w-[180px] shrink-0">
                    <Select value={selectedAgentId} onValueChange={setSelectedAgentId}>
                      <SelectTrigger>
                        <SelectValue placeholder="选择 Agent" />
                      </SelectTrigger>
                      <SelectContent>
                        {visibleAgents.map((agent) => (
                          <SelectItem key={agent.id} value={agent.id}>
                            {agent.name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <Textarea
                    value={composeText}
                    onChange={(event) => setComposeText(event.target.value)}
                    className="min-h-[84px]"
                    placeholder="在团队频道里继续协作，支持共享历史和线程回复。"
                  />
                  <Button onClick={() => void handleSend()} disabled={sending || !composeText.trim()}>
                    <Send className="mr-1 h-4 w-4" />发送
                  </Button>
                </div>
                {error ? <div className="mt-2 text-[12px] text-destructive">{error}</div> : null}
              </div>
            </>
          ) : (
            <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
              选择一个团队频道开始协作。
            </div>
          )}
        </div>

        {!isMobile && threadRootId && threadRootMessage ? (
          <div className="w-[360px] shrink-0 border-l border-border/60 bg-muted/8">
            <div className="flex items-center justify-between border-b border-border/60 px-4 py-3">
              <div>
                <div className="text-[13px] font-medium text-foreground">线程回复</div>
                <div className="text-[11px] text-muted-foreground">围绕当前消息继续展开</div>
              </div>
              <button type="button" onClick={() => setThreadRootId(null)} className="text-[12px] text-muted-foreground hover:text-foreground">
                关闭
              </button>
            </div>
            <div className="h-[calc(100%-136px)] overflow-y-auto px-4 py-4">
              <div className="space-y-4">
                {threadRootMessage ? renderMessage(threadRootMessage) : null}
                {threadMessages.map(renderMessage)}
              </div>
            </div>
            <div className="border-t border-border/60 px-4 py-3">
              <Textarea
                value={threadComposeText}
                onChange={(event) => setThreadComposeText(event.target.value)}
                className="min-h-[88px]"
                placeholder="在线程里继续回复"
              />
              <div className="mt-2 flex justify-end">
                <Button onClick={() => void handleSend(threadRootId)} disabled={sending || !threadComposeText.trim()}>
                  <MessageSquareReply className="mr-1 h-4 w-4" />回复线程
                </Button>
              </div>
            </div>
          </div>
        ) : null}
      </div>

      <BottomSheetPanel open={isMobile && !!threadRootId} onOpenChange={(open) => !open && setThreadRootId(null)} title="线程回复" description="围绕当前消息继续展开" fullHeight>
        <div className="space-y-4">
          {threadRootMessage ? renderMessage(threadRootMessage) : null}
          {threadMessages.map(renderMessage)}
          <Textarea
            value={threadComposeText}
            onChange={(event) => setThreadComposeText(event.target.value)}
            className="min-h-[96px]"
            placeholder="在线程里继续回复"
          />
          <div className="flex justify-end">
            <Button onClick={() => void handleSend(threadRootId)} disabled={sending || !threadComposeText.trim()}>
              <MessageSquareReply className="mr-1 h-4 w-4" />回复线程
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
          <div className="flex justify-end">
            <Button onClick={() => void handleSaveChannel()}>
              <Save className="mr-1 h-4 w-4" />保存频道设置
            </Button>
          </div>
        </div>
      </BottomSheetPanel>

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
