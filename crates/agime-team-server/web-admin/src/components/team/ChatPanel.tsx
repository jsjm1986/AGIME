import { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, Zap, Puzzle, MessageCircle, Plus, ArrowLeft } from 'lucide-react';
import { agentApi, TeamAgent } from '../../api/agent';
import { ChatSessionList } from '../chat/ChatSessionList';
import { ChatConversation } from '../chat/ChatConversation';
import { AgentSelector } from '../chat/AgentSelector';
import { Button } from '../ui/button';
import { useIsMobile } from '../../hooks/useMediaQuery';

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

  const handleSelectSession = useCallback((sid: string) => {
    setSelectedSessionId(sid);
  }, []);

  const handleNewChat = useCallback(() => {
    setSelectedSessionId(null);
    setSelectedAgent(null);
  }, []);

  const handleSessionCreated = useCallback((sid: string) => {
    setSelectedSessionId(sid);
  }, []);

  const handleAgentSelect = useCallback((agent: TeamAgent) => {
    setSelectedAgent(agent);
  }, []);

  const handleSessionRemoved = useCallback(() => {
    setSelectedSessionId(null);
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
        onSelectSession={handleSelectSession}
        onSessionRemoved={handleSessionRemoved}
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
          onSessionCreated={handleSessionCreated}
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
    <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground gap-6 p-8">
      <div className="flex flex-col items-center gap-2">
        <MessageCircle className="h-10 w-10 text-muted-foreground/40" />
        <div className="text-lg font-medium">{t('chat.title')}</div>
        <p className="text-sm text-center max-w-md">{t('chat.selectAgentHint')}</p>
      </div>

      {agents.length > 0 && (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 w-full max-w-lg">
          {agents.map(agent => {
            const skillCount = agent.assigned_skills?.filter(s => s.enabled).length || 0;
            const extCount = agent.enabled_extensions?.filter(e => e.enabled).length || 0;
            return (
              <button
                key={agent.id}
                onClick={() => onAgentSelect(agent)}
                className="text-left border rounded-lg p-4 hover:bg-accent hover:border-primary/30 transition-colors group"
              >
                <div className="flex items-center gap-2 mb-2">
                  <div className="w-8 h-8 rounded-full bg-muted-foreground/15 flex items-center justify-center shrink-0">
                    <Bot className="w-4 h-4 text-muted-foreground" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-1.5">
                      <span className="text-sm font-medium text-foreground truncate">{agent.name}</span>
                      <span className={`h-2 w-2 rounded-full shrink-0 ${
                        agent.status === 'running' ? 'bg-green-500' :
                        agent.status === 'error' ? 'bg-red-500' :
                        agent.status === 'paused' ? 'bg-amber-500' : 'bg-slate-400'
                      }`} />
                    </div>
                    {agent.model && (
                      <span className="text-[11px] text-muted-foreground">{agent.model}</span>
                    )}
                  </div>
                </div>
                {agent.description && (
                  <p className="text-xs text-muted-foreground line-clamp-2 mb-2">{agent.description}</p>
                )}
                {(skillCount > 0 || extCount > 0) && (
                  <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                    {skillCount > 0 && (
                      <span className="inline-flex items-center gap-0.5">
                        <Zap className="h-3 w-3 text-amber-500" />
                        {skillCount} {t('chat.skills', 'skills')}
                      </span>
                    )}
                    {extCount > 0 && (
                      <span className="inline-flex items-center gap-0.5">
                        <Puzzle className="h-3 w-3 text-blue-500" />
                        {extCount} {t('chat.extensions', 'extensions')}
                      </span>
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
