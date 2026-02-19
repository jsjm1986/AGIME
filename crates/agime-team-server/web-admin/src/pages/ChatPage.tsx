import { useState, useCallback, useEffect } from 'react';
import { useParams, useNavigate, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { TeamAgent } from '../api/agent';
import { ChatSessionList } from '../components/chat/ChatSessionList';
import { ChatConversation } from '../components/chat/ChatConversation';
import { AgentSelector } from '../components/chat/AgentSelector';

export default function ChatPage() {
  const { teamId, sessionId: urlSessionId } = useParams<{
    teamId: string;
    sessionId?: string;
  }>();
  const navigate = useNavigate();
  const location = useLocation();
  const locationState = location.state as { attachedDocumentIds?: string[] } | null;

  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    urlSessionId || null
  );
  const [selectedAgent, setSelectedAgent] = useState<TeamAgent | null>(null);
  const [filterAgentId, setFilterAgentId] = useState<string | undefined>();

  // C5 fix: All hooks must be called before any conditional return
  const handleSelectSession = useCallback(
    (sid: string) => {
      setSelectedSessionId(sid);
      navigate(`/teams/${teamId}/chat/${sid}`, { replace: true });
    },
    [teamId, navigate]
  );

  const handleSessionCreated = useCallback(
    (sid: string) => {
      setSelectedSessionId(sid);
      navigate(`/teams/${teamId}/chat/${sid}`, { replace: true });
    },
    [teamId, navigate]
  );

  const handleAgentSelect = useCallback((agent: TeamAgent) => {
    setSelectedAgent(agent);
  }, []);

  const handleSessionRemoved = useCallback(
    (_sid: string) => {
      setSelectedSessionId(null);
      navigate(`/teams/${teamId}/chat`, { replace: true });
    },
    [teamId, navigate]
  );

  // H10 fix: Sync URL sessionId changes (e.g. browser back/forward)
  useEffect(() => {
    setSelectedSessionId(urlSessionId || null);
  }, [urlSessionId]);

  if (!teamId) return null;

  return (
    <div className="flex h-[calc(100vh-64px)]">
      {/* Left panel: session list */}
      <div className="w-[260px] border-r flex flex-col shrink-0">
        {/* Agent filter */}
        <div className="p-3 border-b">
          <AgentSelector
            teamId={teamId}
            selectedAgentId={filterAgentId || null}
            onSelect={(agent) => {
              setFilterAgentId(agent.id);
              setSelectedAgent(agent);
            }}
            onClear={() => {
              setFilterAgentId(undefined);
            }}
          />
        </div>

        <ChatSessionList
          teamId={teamId}
          agentId={filterAgentId}
          selectedSessionId={selectedSessionId}
          onSelectSession={handleSelectSession}
          onSessionRemoved={handleSessionRemoved}
        />
      </div>

      {/* Right panel: conversation */}
      <div className="flex-1 flex flex-col min-w-0">
        {selectedSessionId ? (
          <ChatConversation
            sessionId={selectedSessionId}
            agentId={selectedAgent?.id || ''}
            agentName={selectedAgent?.name || 'Agent'}
            teamId={teamId}
            onSessionCreated={handleSessionCreated}
          />
        ) : selectedAgent ? (
          <ChatConversation
            sessionId={null}
            agentId={selectedAgent.id}
            agentName={selectedAgent.name}
            teamId={teamId}
            initialAttachedDocIds={locationState?.attachedDocumentIds}
            onSessionCreated={handleSessionCreated}
          />
        ) : (
          <EmptyState teamId={teamId} onAgentSelect={handleAgentSelect} />
        )}
      </div>
    </div>
  );
}

function EmptyState({
  teamId,
  onAgentSelect,
}: {
  teamId: string;
  onAgentSelect: (agent: TeamAgent) => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground gap-4 p-8">
      <div className="text-lg font-medium">
        {t('chat.title', 'Chat')}
      </div>
      <p className="text-sm text-center max-w-md">
        {t('chat.selectAgentHint', 'Select an agent from the left panel to start a new conversation.')}
      </p>
      <div className="w-64">
        <AgentSelector
          teamId={teamId}
          selectedAgentId={null}
          onSelect={onAgentSelect}
        />
      </div>
    </div>
  );
}
