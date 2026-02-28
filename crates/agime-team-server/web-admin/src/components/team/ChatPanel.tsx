import { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, ArrowLeft } from 'lucide-react';
import { agentApi, TeamAgent, BUILTIN_EXTENSIONS } from '../../api/agent';
import { AgentAvatar } from '../agent/AvatarPicker';
import { ChatSessionList } from '../chat/ChatSessionList';
import { ChatConversation } from '../chat/ChatConversation';
import { AgentSelector } from '../chat/AgentSelector';
import { Button } from '../ui/button';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { StatusBadge, AGENT_STATUS_MAP } from '../ui/status-badge';

const STATUS_RING: Record<string, string> = {
  idle: 'ring-2 ring-status-success-text/30',
  running: 'ring-2 ring-status-info-text/40 animate-pulse',
  paused: 'ring-2 ring-status-warning-text/30',
  error: 'ring-2 ring-status-error-text/40',
};

interface ChatPanelProps {
  teamId: string;
  initialAgent?: TeamAgent | null;
}

export function ChatPanel({ teamId, initialAgent }: ChatPanelProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(initialAgent || null);
  const [filterAgentId, setFilterAgentId] = useState<string | undefined>(initialAgent?.id);

  // Sync when initialAgent prop changes (e.g. from AgentManagePanel "Chat" button)
  useEffect(() => {
    if (initialAgent) {
      setSelectedAgent(initialAgent);
      setFilterAgentId(initialAgent.id);
      setSelectedSessionId(null);
    }
  }, [initialAgent]);

  const handleNewChat = useCallback(() => {
    setSelectedSessionId(null);
    setSelectedAgent(null);
  }, []);

  const handleAgentSelect = useCallback((agent: TeamAgent) => {
    setSelectedAgent(agent);
  }, []);

  const mobileShowConversation = isMobile && (selectedSessionId || selectedAgent);

  const sessionListPanel = (
    <div className={isMobile ? 'flex-1 flex flex-col bg-muted/20' : 'w-[260px] border-r border-border/50 flex flex-col shrink-0 bg-muted/20'}>
      <div className="p-2 flex items-center gap-1.5">
        <div className="flex-1 min-w-0">
          <AgentSelector
            teamId={teamId}
            selectedAgentId={filterAgentId || null}
            onSelect={(agent) => {
              setFilterAgentId(agent.id);
              setSelectedAgent(agent);
            }}
            onClear={() => setFilterAgentId(undefined)}
            compact
          />
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 shrink-0"
          onClick={handleNewChat}
          title={t('chat.newChat', 'New Chat')}
        >
          <Plus className="h-4 w-4" />
        </Button>
      </div>
      <ChatSessionList
        teamId={teamId}
        agentId={filterAgentId}
        selectedSessionId={selectedSessionId}
        onSelectSession={setSelectedSessionId}
        onSessionRemoved={() => setSelectedSessionId(null)}
      />
    </div>
  );

  const conversationPanel = (
    <div className="flex-1 flex flex-col min-w-0 min-h-0">
      {isMobile && (
        <div className="flex items-center gap-2 px-3 py-2 border-b">
          <button onClick={handleNewChat} className="p-1 rounded-md hover:bg-muted">
            <ArrowLeft className="w-4 h-4" />
          </button>
          <span className="text-sm font-medium truncate">{selectedAgent?.name || t('chat.title')}</span>
        </div>
      )}
      {selectedAgent ? (
        <ChatConversation
          sessionId={selectedSessionId}
          agentId={selectedAgent.id}
          agentName={selectedAgent.name}
          agent={selectedAgent}
          teamId={teamId}
          onSessionCreated={setSelectedSessionId}
        />
      ) : (
        <ChatEmptyState teamId={teamId} onAgentSelect={handleAgentSelect} />
      )}
    </div>
  );

  if (isMobile) {
    return (
      <div className="flex flex-col h-[calc(100vh-40px)]">
        {mobileShowConversation ? conversationPanel : sessionListPanel}
      </div>
    );
  }

  return (
    <div className="flex h-[calc(100vh-40px)]">
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
  teamId,
  onAgentSelect,
}: {
  teamId: string;
  onAgentSelect: (agent: TeamAgent) => void;
}) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<TeamAgent[]>([]);

  useEffect(() => {
    agentApi.listAgents(teamId).then(res => setAgents(res.items || [])).catch(() => {});
  }, [teamId]);

  return (
    <div className="flex-1 flex flex-col items-center justify-start overflow-y-auto p-6 sm:p-10">
      <p className="text-sm text-muted-foreground mb-6">{t('chat.selectAgentHint')}</p>

      {agents.length > 0 && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3 w-full max-w-3xl">
          {agents.map(agent => {
            const st = STATUS_RING[agent.status] || STATUS_RING.idle;
            const extCount = getExtensionNames(agent).length;
            const skillCount = getSkillNames(agent).length;

            return (
              <button
                key={agent.id}
                onClick={() => onAgentSelect(agent)}
                className="flex flex-col items-center text-center rounded-xl border border-border/60 p-4 hover:border-primary/30 hover:shadow-sm transition-all group"
              >
                {/* Avatar + Status ring */}
                <div className="mb-2.5">
                  <AgentAvatar avatar={agent.avatar} name={agent.name} className={`w-14 h-14 bg-muted ${st}`} iconSize="w-7 h-7" />
                </div>

                {/* Name + Status */}
                <div className="flex items-center gap-1.5 mb-1">
                  <span className="text-sm font-semibold text-foreground truncate max-w-[140px]">{agent.name}</span>
                  <StatusBadge status={AGENT_STATUS_MAP[agent.status]} className="shrink-0">
                    {t(`agent.status.${agent.status}`)}
                  </StatusBadge>
                </div>

                {/* Model */}
                {agent.model && (
                  <span className="inline-block text-caption font-mono text-blue-600 dark:text-blue-400 bg-blue-50 dark:bg-blue-950/30 rounded px-1.5 py-0.5 mb-1.5">
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
                      <span className="text-violet-600 dark:text-violet-400">{extCount} {t('chat.extensions')}</span>
                    )}
                    {extCount > 0 && skillCount > 0 && <span className="text-muted-foreground/30">·</span>}
                    {skillCount > 0 && (
                      <span className="text-amber-600 dark:text-amber-400">{skillCount} {t('chat.skills')}</span>
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
