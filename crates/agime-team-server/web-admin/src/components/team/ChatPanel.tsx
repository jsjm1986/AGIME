import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, ArrowLeft, ChevronRight, Clock3, History, MessageSquareText, Users } from 'lucide-react';
import { TeamAgent, BUILTIN_EXTENSIONS } from '../../api/agent';
import { chatApi, ChatSession } from '../../api/chat';
import { AgentAvatar } from '../agent/AvatarPicker';
import { ChatSessionList } from '../chat/ChatSessionList';
import { ChatConversation } from '../chat/ChatConversation';
import { AgentSelector } from '../chat/AgentSelector';
import { Button } from '../ui/button';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { StatusBadge, AGENT_STATUS_MAP } from '../ui/status-badge';
import { fetchVisibleChatAgents, isVisibleChatAgent } from '../chat/visibleChatAgents';
import type { ChatInputComposeRequest } from '../chat/ChatInput';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { formatRelativeTime } from '../../utils/format';

const STATUS_RING: Record<string, string> = {
  idle: 'ring-2 ring-status-success-text/30',
  running: 'ring-2 ring-status-info-text/40 animate-pulse',
  paused: 'ring-2 ring-status-warning-text/30',
  error: 'ring-2 ring-status-error-text/40',
};

type MobileChatView = 'home' | 'sessions' | 'conversation';

function getDefaultVisibleAgent(
  initialAgent: TeamAgent | null | undefined,
  filterAgentId: string | undefined,
  visibleAgents: TeamAgent[],
) {
  if (initialAgent && isVisibleChatAgent(visibleAgents, initialAgent.id)) {
    return initialAgent;
  }
  if (filterAgentId) {
    const matched = visibleAgents.find((agent) => agent.id === filterAgentId);
    if (matched) {
      return matched;
    }
  }
  return visibleAgents[0] || null;
}

interface ChatPanelProps {
  teamId: string;
  initialAgent?: TeamAgent | null;
  launchContext?: ChatLaunchContext | null;
}

export interface ChatLaunchContext {
  requestId: string;
  attachedDocumentIds?: string[];
  composeRequest?: ChatInputComposeRequest | null;
}

export function ChatPanel({ teamId, initialAgent, launchContext }: ChatPanelProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [selectedSessionMeta, setSelectedSessionMeta] = useState<ChatSession | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(initialAgent || null);
  const [filterAgentId, setFilterAgentId] = useState<string | undefined>(initialAgent?.id);
  const [visibleAgents, setVisibleAgents] = useState<TeamAgent[]>([]);
  const [visibleAgentsReady, setVisibleAgentsReady] = useState(false);
  const [activeLaunchContext, setActiveLaunchContext] = useState<ChatLaunchContext | null>(launchContext || null);
  const [mobileView, setMobileView] = useState<MobileChatView>('home');
  const [recentSessionSummary, setRecentSessionSummary] = useState<ChatSession | null>(null);
  const [recentSessionLoading, setRecentSessionLoading] = useState(false);
  const [agentSheetOpen, setAgentSheetOpen] = useState(false);
  const lastLaunchRequestIdRef = useRef<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setVisibleAgentsReady(false);
    const loadVisibleAgents = async () => {
      try {
        const agents = await fetchVisibleChatAgents(teamId);
        if (!cancelled) {
          setVisibleAgents(agents);
          setVisibleAgentsReady(true);
        }
      } catch (error) {
        console.error('Failed to load visible chat agents:', error);
        if (!cancelled) {
          setVisibleAgents([]);
          setVisibleAgentsReady(true);
        }
      }
    };

    loadVisibleAgents();
    return () => {
      cancelled = true;
    };
  }, [teamId]);

  // Sync when initialAgent prop changes (e.g. from AgentManagePanel "Chat" button)
  useEffect(() => {
    if (initialAgent) {
      setSelectedAgent(initialAgent);
      setFilterAgentId(isVisibleChatAgent(visibleAgents, initialAgent.id) ? initialAgent.id : undefined);
      setSelectedSessionId(null);
      setSelectedSessionMeta(null);
    }
  }, [initialAgent, visibleAgents]);

  useEffect(() => {
    if (!launchContext?.requestId) {
      return;
    }
    if (lastLaunchRequestIdRef.current === launchContext.requestId) {
      return;
    }
    lastLaunchRequestIdRef.current = launchContext.requestId;
    setActiveLaunchContext(launchContext);
    setSelectedSessionId(null);
    setSelectedSessionMeta(null);
    if (initialAgent) {
      setSelectedAgent(initialAgent);
      setFilterAgentId(isVisibleChatAgent(visibleAgents, initialAgent.id) ? initialAgent.id : undefined);
    }
    if (isMobile) {
      setMobileView('conversation');
    }
  }, [initialAgent, isMobile, launchContext, visibleAgents]);

  useEffect(() => {
    if (!isMobile) {
      return;
    }
    setMobileView('home');
    setRecentSessionSummary(null);
    setRecentSessionLoading(false);
  }, [teamId, isMobile]);

  const resolveDefaultAgent = useCallback(() => {
    return getDefaultVisibleAgent(initialAgent, filterAgentId, visibleAgents);
  }, [filterAgentId, initialAgent, visibleAgents]);

  useEffect(() => {
    if (!isMobile || !visibleAgentsReady || visibleAgents.length === 0) {
      return;
    }
    if (selectedAgent && isVisibleChatAgent(visibleAgents, selectedAgent.id)) {
      return;
    }
    const defaultAgent = resolveDefaultAgent();
    if (defaultAgent) {
      setSelectedAgent(defaultAgent);
    }
  }, [isMobile, resolveDefaultAgent, selectedAgent, visibleAgents, visibleAgentsReady]);

  useEffect(() => {
    if (!isMobile || !visibleAgentsReady) {
      return;
    }

    const agentId = selectedAgent?.id || resolveDefaultAgent()?.id;
    if (!agentId) {
      setRecentSessionSummary(null);
      setRecentSessionLoading(false);
      return;
    }

    let cancelled = false;
    setRecentSessionLoading(true);
    const loadRecentSession = async () => {
      try {
        const recentSessions = await chatApi.listSessions(teamId, agentId, 1, 1, undefined, false);
        if (!cancelled) {
          setRecentSessionSummary(recentSessions[0] || null);
        }
      } catch (error) {
        console.error('Failed to load recent mobile session:', error);
        if (!cancelled) {
          setRecentSessionSummary(null);
        }
      } finally {
        if (!cancelled) {
          setRecentSessionLoading(false);
        }
      }
    };

    void loadRecentSession();
    return () => {
      cancelled = true;
    };
  }, [isMobile, resolveDefaultAgent, selectedAgent?.id, teamId, visibleAgentsReady]);

  const handleStartFreshChat = useCallback(() => {
    setSelectedSessionId(null);
    setSelectedSessionMeta(null);
    setSelectedAgent(resolveDefaultAgent());
    setActiveLaunchContext(null);
    if (isMobile) {
      setMobileView('conversation');
    }
  }, [isMobile, resolveDefaultAgent]);

  const handleBackToHome = useCallback(() => {
    if (!isMobile) {
      return;
    }
    if (selectedSessionMeta) {
      setRecentSessionSummary(selectedSessionMeta);
    }
    setMobileView('home');
  }, [isMobile, selectedSessionMeta]);

  const handleOpenSessionList = useCallback(() => {
    if (!isMobile) {
      return;
    }
    setMobileView('sessions');
  }, [isMobile]);

  const handleSelectSession = useCallback((session: ChatSession) => {
    const matchedAgent = visibleAgents.find((agent) => agent.id === session.agent_id) || null;
    setSelectedSessionId(session.session_id);
    setSelectedSessionMeta(session);
    setSelectedAgent(matchedAgent);
    setFilterAgentId(session.agent_id);
    setActiveLaunchContext(null);
    setRecentSessionSummary(session);
    if (isMobile) {
      setMobileView('conversation');
    }
  }, [isMobile, visibleAgents]);

  const handleAgentSelect = useCallback((agent: TeamAgent) => {
    setSelectedAgent(agent);
    setFilterAgentId(agent.id);
    setSelectedSessionMeta(null);
    setSelectedSessionId(null);
    setActiveLaunchContext(null);
  }, []);

  const handleSessionCreated = useCallback((sessionId: string) => {
    setSelectedSessionId(sessionId);
    const createdAt = new Date().toISOString();
    if (selectedAgent) {
      const nextSessionMeta = {
        session_id: sessionId,
        agent_id: selectedAgent.id,
        agent_name: selectedAgent.name,
        title: undefined,
        last_message_preview: undefined,
        last_message_at: undefined,
        message_count: 0,
        status: 'active',
        pinned: false,
        created_at: createdAt,
      } satisfies ChatSession;
      setSelectedSessionMeta(nextSessionMeta);
      setRecentSessionSummary(nextSessionMeta);
    }
    setActiveLaunchContext(null);
    if (isMobile) {
      setMobileView('conversation');
    }
  }, [isMobile, selectedAgent]);

  const activeSessionAgentId = selectedAgent?.id || selectedSessionMeta?.agent_id || '';
  const activeSessionAgentName = selectedAgent?.name || selectedSessionMeta?.agent_name || t('chat.title', '聊天');
  const mobileHomeAgent = selectedAgent || resolveDefaultAgent();
  const mobileRecentSession = recentSessionSummary;
  const conversationInstanceKey = activeSessionAgentId || 'chat-conversation';

  const handleListAgentSelect = useCallback((agent: TeamAgent) => {
    setFilterAgentId(agent.id);
    setSelectedAgent(agent);
    setActiveLaunchContext(null);
  }, []);

  const handleListAgentClear = useCallback(() => {
    setFilterAgentId(undefined);
    setActiveLaunchContext(null);
  }, []);

  const handleSessionRemoved = useCallback((sessionId: string) => {
    setSelectedSessionId((current) => (current === sessionId ? null : current));
    setSelectedSessionMeta((current) => (current?.session_id === sessionId ? null : current));
    setRecentSessionSummary((current) => (current?.session_id === sessionId ? null : current));
    if (isMobile) {
      setMobileView('home');
    }
  }, [isMobile]);

  const sessionListPanel = (
    <div className="flex w-full shrink-0 flex-col border-r border-border/60 bg-muted/12 md:w-[min(32vw,248px)] lg:w-[min(26vw,260px)]">
      <div className="p-2 flex items-center gap-1.5">
        <div className="flex-1 min-w-0">
          <AgentSelector
            teamId={teamId}
            selectedAgentId={filterAgentId || null}
            onSelect={handleListAgentSelect}
            onClear={handleListAgentClear}
            compact
          />
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 shrink-0"
          onClick={handleStartFreshChat}
          title={t('chat.newChat', 'New Chat')}
        >
          <Plus className="h-4 w-4" />
        </Button>
      </div>
      <ChatSessionList
        teamId={teamId}
        agentId={filterAgentId}
        selectedSessionId={selectedSessionId}
        onSelectSession={handleSelectSession}
        onSessionRemoved={handleSessionRemoved}
      />
    </div>
  );

  const conversationPanel = (
    <div className="flex-1 flex flex-col min-w-0 min-h-0">
      {selectedSessionId || selectedAgent || selectedSessionMeta ? (
        <ChatConversation
          key={conversationInstanceKey}
          sessionId={selectedSessionId}
          agentId={activeSessionAgentId}
          agentName={activeSessionAgentName}
          agent={selectedAgent}
          headerVariant={isMobile ? 'compact' : isConversationMode && isMobileWorkspace ? 'compact' : 'default'}
          headerLeading={
            isMobile ? (
              <button
                type="button"
                onClick={handleBackToHome}
                className="inline-flex h-8 w-8 items-center justify-center rounded-2xl border border-border/60 bg-background text-muted-foreground transition-colors hover:bg-muted/45 hover:text-foreground"
                title={t('common.back', '返回')}
              >
                <ArrowLeft className="h-4 w-4" />
              </button>
            ) : null
          }
          headerActions={
            isMobile ? (
              <button
                type="button"
                onClick={handleOpenSessionList}
                className="inline-flex h-8 items-center gap-1 rounded-full border border-border/60 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45"
                title={t('chat.sessions', '会话')}
              >
                <History className="h-3.5 w-3.5 text-muted-foreground" />
                <span>{t('chat.sessions', '会话')}</span>
              </button>
            ) : null
          }
          composerActions={
            isConversationMode && isMobileWorkspace ? (
              <button
                type="button"
                onClick={handleOpenSessionList}
                className="inline-flex h-9 items-center gap-1 rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45 sm:h-10 sm:text-[12px]"
                title={t('chat.sessions', '会话')}
              >
                <span>{t('chat.sessions', '会话')}</span>
              </button>
            ) : null
          }
          composerCollapsedActions={
            isConversationMode && isMobileWorkspace ? (
              <>
                <button
                  type="button"
                  onClick={handleOpenSessionList}
                  className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
                >
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium text-foreground">
                      {t('chat.sessions', '会话')}
                    </div>
                    <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                      {t('chat.mobileBrowseSessionsHint', '查看最近会话，继续处理已有任务或历史聊天。')}
                    </div>
                  </div>
                </button>
                <button
                  type="button"
                  onClick={() => setAgentSheetOpen(true)}
                  className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
                >
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium text-foreground">
                      {t('chat.selectAgent', '选择 Agent')}
                    </div>
                    <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                      {t('chat.mobileAgentHint', '切换当前 Agent，再继续对话或开始新的工作流。')}
                    </div>
                  </div>
                </button>
                <button
                  type="button"
                  onClick={handleStartFreshChat}
                  className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"
                >
                  <div className="min-w-0">
                    <div className="text-[13px] font-medium text-foreground">
                      {t('chat.newChat', '新对话')}
                    </div>
                    <div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">
                      {t('chat.newChatHint', '清空当前上下文，开始一段新的对话。')}
                    </div>
                  </div>
                </button>
              </>
            ) : null
          }
          teamId={teamId}
          initialAttachedDocIds={selectedSessionId ? undefined : activeLaunchContext?.attachedDocumentIds}
          composeRequest={selectedSessionId ? null : activeLaunchContext?.composeRequest || null}
          onSessionCreated={handleSessionCreated}
        />
      ) : (
        <ChatEmptyState onAgentSelect={handleAgentSelect} visibleAgents={visibleAgents} />
      )}
    </div>
  );

  const mobileConversationView = (
    <div className="chat-font-cap flex h-[calc(100dvh-40px)] min-h-0 flex-col overflow-hidden bg-background">
      {conversationPanel}
    </div>
  );

  const mobileSessionListView = (
    <div className="chat-font-cap flex h-[calc(100dvh-40px)] min-h-0 flex-col overflow-hidden bg-background">
      <div className="flex items-center gap-2 border-b border-border/60 px-4 py-3">
        <button
          type="button"
          onClick={handleBackToHome}
          className="rounded-md p-1.5 transition-colors hover:bg-muted/70"
        >
          <ArrowLeft className="h-4 w-4" />
        </button>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-foreground">{t('chat.sessions', '会话')}</div>
          <div className="truncate text-[11px] text-muted-foreground">
            {t('chat.mobileBrowseSessionsHint', '查看最近会话，继续处理已有任务或历史聊天。')}
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 shrink-0"
          onClick={handleStartFreshChat}
          title={t('chat.newChat', 'New Chat')}
        >
          <Plus className="h-4 w-4" />
        </Button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        <div className="border-b border-border/60 p-3">
          <AgentSelector
            teamId={teamId}
            selectedAgentId={filterAgentId || null}
            onSelect={handleListAgentSelect}
            onClear={handleListAgentClear}
            compact
          />
        </div>
        <div className="min-h-0 flex-1">
          <ChatSessionList
            teamId={teamId}
            agentId={filterAgentId}
            selectedSessionId={selectedSessionId}
            onSelectSession={handleSelectSession}
            onSessionRemoved={handleSessionRemoved}
          />
        </div>
      </div>
    </div>
  );

  const mobileHomeView = (
    <div className="chat-font-cap flex h-[calc(100dvh-40px)] min-h-0 flex-col overflow-hidden bg-background">
      <div className="border-b border-border/60 px-4 py-4">
        <div className="text-[11px] font-medium uppercase tracking-[0.16em] text-muted-foreground">
          {t('chat.mobileHomeEyebrow', '对话首页')}
        </div>
        <div className="mt-1 text-[22px] font-semibold tracking-[-0.02em] text-foreground">
          {t('chat.mobileHomeTitle', '继续最近对话或开始新任务')}
        </div>
        <p className="mt-1 max-w-[28rem] text-[13px] leading-5 text-muted-foreground">
          {t('chat.mobileHomeDescription', '移动端优先进入对话首页，再决定继续旧会话、切换 Agent 或开始新对话。')}
        </p>
      </div>

      <div className="flex-1 overflow-y-auto px-4 pb-[calc(env(safe-area-inset-bottom)+24px)] pt-4">
        <div className={`space-y-4 ${isConversationMode ? 'max-w-none' : 'max-w-xl'}`}>
          {mobileHomeAgent ? (
            <section className="rounded-[26px] border border-border/70 bg-card px-4 py-4 shadow-[0_14px_28px_-24px_rgba(15,23,42,0.35)]">
              <div className="flex items-start gap-3">
                <AgentAvatar
                  avatar={mobileHomeAgent.avatar}
                  name={mobileHomeAgent.name}
                  className={`h-12 w-12 bg-muted ${STATUS_RING[mobileHomeAgent.status] || STATUS_RING.idle}`}
                  iconSize="h-6 w-6"
                />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <div className="truncate text-[15px] font-semibold text-foreground">
                      {mobileHomeAgent.name}
                    </div>
                    <StatusBadge status={AGENT_STATUS_MAP[mobileHomeAgent.status]}>
                      {t(`agent.status.${mobileHomeAgent.status}`)}
                    </StatusBadge>
                  </div>
                  {mobileHomeAgent.description ? (
                    <p className="mt-1 line-clamp-2 text-[12px] leading-5 text-muted-foreground">
                      {mobileHomeAgent.description}
                    </p>
                  ) : null}
                  <div className="mt-3 flex flex-wrap items-center gap-3 text-[11px] text-muted-foreground">
                    <span className="inline-flex items-center gap-1.5">
                      <Users className="h-3.5 w-3.5" />
                      {t('chat.mobileHomeAgentLabel', '当前 Agent')}
                    </span>
                    <span>
                      {getExtensionNames(mobileHomeAgent).length} {t('chat.extensions', '扩展')}
                    </span>
                    <span>
                      {getSkillNames(mobileHomeAgent).length} {t('chat.skills', '技能')}
                    </span>
                  </div>
                </div>
              </div>

              <div className="mt-4 grid gap-2">
                <button
                  type="button"
                  onClick={handleStartFreshChat}
                  className="inline-flex w-full items-center justify-center rounded-[16px] bg-primary px-4 py-3 text-[13px] font-semibold text-primary-foreground shadow-sm transition-transform hover:translate-y-[-1px]"
                >
                  <MessageSquareText className="mr-2 h-4 w-4" />
                  {mobileRecentSession
                    ? t('chat.mobileStartNewConversation', '开始新对话')
                    : t('chat.mobileStartConversation', '开始对话')}
                </button>
                <div className="grid grid-cols-2 gap-2">
                  <button
                    type="button"
                    onClick={handleOpenSessionList}
                    className="inline-flex items-center justify-center rounded-[16px] border border-border/70 bg-background px-4 py-2.5 text-[12px] font-medium text-foreground transition-colors hover:bg-muted/45"
                  >
                    <History className="mr-2 h-4 w-4" />
                    {t('chat.mobileViewAllSessions', '查看全部会话')}
                  </button>
                  <button
                    type="button"
                    onClick={() => setAgentSheetOpen(true)}
                    className="inline-flex items-center justify-center rounded-[16px] border border-border/70 bg-background px-4 py-2.5 text-[12px] font-medium text-foreground transition-colors hover:bg-muted/45"
                  >
                    <Users className="mr-2 h-4 w-4" />
                    {t('chat.selectAgent', '选择 Agent')}
                  </button>
                </div>
              </div>
            </section>
          ) : (
            <div className="rounded-[26px] border border-border/70 bg-card px-4 py-5 text-[13px] text-muted-foreground">
              {visibleAgentsReady
                ? t('chat.selectAgentHint', '从左侧面板选择一个 Agent 开始新对话。')
                : t('common.loading', '加载中...')}
            </div>
          )}

          <section className="rounded-[26px] border border-border/70 bg-card px-4 py-4 shadow-[0_12px_26px_-24px_rgba(15,23,42,0.35)]">
            <div className="flex items-start gap-3">
              <div className="inline-flex h-11 w-11 shrink-0 items-center justify-center rounded-[16px] border border-border/70 bg-muted/35 text-muted-foreground">
                <Clock3 className="h-4.5 w-4.5" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-medium text-foreground">
                  {t('chat.mobileRecentConversation', '最近会话')}
                </div>
                {recentSessionLoading ? (
                  <div className="mt-2 text-[12px] text-muted-foreground">
                    {t('chat.loadingRecentSession', '正在准备最近对话')}
                  </div>
                ) : mobileRecentSession ? (
                  <>
                    <div className="mt-1 truncate text-[15px] font-semibold text-foreground">
                      {mobileRecentSession.title || activeSessionAgentName}
                    </div>
                    <div className="mt-1 text-[12px] text-muted-foreground">
                      {formatRelativeTime(
                        mobileRecentSession.last_message_at || mobileRecentSession.created_at,
                        t,
                      )}
                      {mobileRecentSession.message_count
                        ? ` · ${mobileRecentSession.message_count}${t('chat.messagesShort', '条消息')}`
                        : ''}
                    </div>
                    {mobileRecentSession.last_message_preview ? (
                      <p className="mt-2 line-clamp-2 text-[12px] leading-5 text-muted-foreground">
                        {mobileRecentSession.last_message_preview}
                      </p>
                    ) : null}
                    <button
                      type="button"
                      onClick={() => handleSelectSession(mobileRecentSession)}
                      className="mt-3 inline-flex items-center rounded-full border border-border/70 bg-background px-3 py-2 text-[12px] font-medium text-foreground transition-colors hover:bg-muted/45"
                    >
                      {t('chat.mobileContinueRecent', '继续最近会话')}
                      <ChevronRight className="ml-1 h-4 w-4" />
                    </button>
                  </>
                ) : (
                  <div className="mt-2 text-[12px] leading-5 text-muted-foreground">
                    {t('chat.mobileNoRecentConversation', '当前 Agent 还没有历史会话，可以直接开始新的对话。')}
                  </div>
                )}
              </div>
            </div>
          </section>
        </div>
      </div>
    </div>
  );

  const agentSheet = (
    <BottomSheetPanel
      open={agentSheetOpen}
      onOpenChange={setAgentSheetOpen}
      title={t('chat.selectAgent', '选择 Agent')}
      description={t(
        'chat.mobileAgentHint',
        '对话模式下优先进入对话，复杂配置和资料留到后续工作流处理。',
      )}
    >
      <div className="space-y-3">
        {visibleAgents.map((agent) => (
          <button
            key={agent.id}
            onClick={() => {
              handleAgentSelect(agent);
              setAgentSheetOpen(false);
            }}
            className={`w-full rounded-[20px] border px-4 py-3 text-left transition-colors ${
              selectedAgent?.id === agent.id
                ? 'border-primary/45 bg-primary/10'
                : 'border-border/70 bg-card/85 hover:bg-accent/30'
            }`}
          >
            <div className="flex items-start gap-3">
              <AgentAvatar
                avatar={agent.avatar}
                name={agent.name}
                className="h-10 w-10 bg-muted"
                iconSize="h-5 w-5"
              />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <div className="truncate text-[13px] font-semibold text-foreground">
                    {agent.name}
                  </div>
                  <StatusBadge status={AGENT_STATUS_MAP[agent.status]}>
                    {t(`agent.status.${agent.status}`)}
                  </StatusBadge>
                </div>
                {agent.description ? (
                  <div className="mt-1 line-clamp-2 text-[12px] leading-5 text-muted-foreground">
                    {agent.description}
                  </div>
                ) : null}
              </div>
            </div>
          </button>
        ))}
      </div>
    </BottomSheetPanel>
  );

  if (isMobile) {
    return (
      <>
        {mobileView === 'conversation' ? mobileConversationView : null}
        {mobileView === 'sessions' ? mobileSessionListView : null}
        {mobileView === 'home' ? mobileHomeView : null}
        {agentSheet}
      </>
    );
  }

  return (
    <div className="chat-font-cap flex h-[calc(100vh-40px)]">
      {sessionListPanel}
      {conversationPanel}
    </div>
  );
}

function getExtensionNames(agent: TeamAgent): string[] {
  const builtinNames = (agent.enabled_extensions || [])
    .filter(e => e.enabled)
    .map(e => BUILTIN_EXTENSIONS.find(b => b.id === e.extension)?.name || e.extension);
  const customNames = (agent.custom_extensions || [])
    .filter(e => e.enabled)
    .map(e => e.name);
  return [...builtinNames, ...customNames];
}

function getSkillNames(agent: TeamAgent): string[] {
  return (agent.assigned_skills || [])
    .filter(s => s.enabled)
    .map(s => s.name);
}

function ChatEmptyState({
  onAgentSelect,
  visibleAgents,
}: {
  onAgentSelect: (agent: TeamAgent) => void;
  visibleAgents: TeamAgent[];
}) {
  const { t } = useTranslation();

  return (
      <div className="flex-1 flex flex-col items-center justify-start overflow-y-auto p-6 sm:p-10">
      <p className="mb-6 max-w-xl text-center text-sm leading-6 text-muted-foreground">{t('chat.selectAgentHint')}</p>

      {visibleAgents.length > 0 && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3 w-full max-w-3xl">
          {visibleAgents.map(agent => {
            const st = STATUS_RING[agent.status] || STATUS_RING.idle;
            const extCount = getExtensionNames(agent).length;
            const skillCount = getSkillNames(agent).length;

            return (
              <button
                key={agent.id}
                onClick={() => onAgentSelect(agent)}
                className="group flex flex-col items-center rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.76] bg-[hsl(var(--ui-surface-panel))/0.7] p-4 text-center transition-all hover:border-[hsl(var(--ui-line-strong))/0.82] hover:bg-[hsl(var(--ui-surface-panel-strong))/0.78]"
              >
                {/* Avatar + Status ring */}
                <div className="mb-2.5">
                  <AgentAvatar avatar={agent.avatar} name={agent.name} className={`h-14 w-14 bg-muted ${st}`} iconSize="w-7 h-7" />
                </div>

                {/* Name + Status */}
                <div className="flex items-center gap-1.5 mb-1">
                  <span className="max-w-full truncate text-sm font-semibold text-foreground sm:max-w-[12rem]">{agent.name}</span>
                  <StatusBadge status={AGENT_STATUS_MAP[agent.status]} className="shrink-0">
                    {t(`agent.status.${agent.status}`)}
                  </StatusBadge>
                </div>

                {/* Model */}
                {agent.model && (
                  <span className="mb-1.5 inline-block rounded-[8px] border border-primary/14 bg-primary/8 px-1.5 py-0.5 font-mono text-caption text-primary">
                    {agent.model}
                  </span>
                )}

                {/* Description */}
                {agent.description && (
                  <p className="text-xs text-muted-foreground leading-relaxed line-clamp-2 w-full">{agent.description}</p>
                )}

                {/* Capabilities */}
                {(extCount > 0 || skillCount > 0) && (
                  <div className="flex items-center gap-2 mt-auto pt-2 text-caption">
                    {extCount > 0 && (
                      <span className="text-status-info-text">{extCount} {t('chat.extensions')}</span>
                    )}
                    {extCount > 0 && skillCount > 0 && <span className="text-muted-foreground/30">·</span>}
                    {skillCount > 0 && (
                      <span className="text-status-warning-text">{skillCount} {t('chat.skills')}</span>
                    )}
                  </div>
                )}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
