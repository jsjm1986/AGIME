import { useState, useEffect, lazy, Suspense, useMemo, useCallback } from 'react';
import { useParams, useNavigate, useSearchParams, Link, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { AppShell } from '../components/layout/AppShell';
import { Skeleton } from '../components/ui/skeleton';
import { TeamProvider } from '../contexts/TeamContext';
import { CreateInviteDialog } from '../components/team/CreateInviteDialog';
import { apiClient } from '../api/client';
import type { TeamWithStats, TeamRole } from '../api/types';
import { agentApi, type TeamAgent } from '../api/agent';
import type { ChatLaunchContext } from '../components/team/ChatPanel';
import { NAV_ITEMS } from '../config/teamNavConfig';

const DocumentsTab = lazy(() =>
  import('../components/team/DocumentsTab').then((module) => ({ default: module.DocumentsTab })),
);
const SmartLogTab = lazy(() =>
  import('../components/team/SmartLogTab').then((module) => ({ default: module.SmartLogTab })),
);
const ChatPanel = lazy(() =>
  import('../components/team/ChatPanel').then((module) => ({ default: module.ChatPanel })),
);
const ToolkitSection = lazy(() =>
  import('../components/team/ToolkitSection').then((module) => ({ default: module.ToolkitSection })),
);
const AgentSection = lazy(() =>
  import('../components/team/AgentSection').then((module) => ({ default: module.AgentSection })),
);
const TeamAdminSection = lazy(() =>
  import('../components/team/TeamAdminSection').then((module) => ({ default: module.TeamAdminSection })),
);
const ExternalUsersTab = lazy(() =>
  import('../components/team/ExternalUsersTab').then((module) => ({ default: module.ExternalUsersTab })),
);
const LaboratorySection = lazy(() =>
  import('../components/team/LaboratorySection').then((module) => ({ default: module.LaboratorySection })),
);
const DigitalAvatarSection = lazy(() =>
  import('../components/team/DigitalAvatarSection').then((module) => ({ default: module.DigitalAvatarSection })),
);

function SectionLoadingFallback() {
  return (
    <div className="space-y-4">
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-64 w-full" />
    </div>
  );
}

export function TeamDetailPage() {
  const { t } = useTranslation();
  const { teamId } = useParams<{ teamId: string }>();
  const navigate = useNavigate();
  const location = useLocation();
  const [searchParams] = useSearchParams();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [inviteDialogOpen, setInviteDialogOpen] = useState(false);
  const [chatAgent, setChatAgent] = useState<TeamAgent | null>(null);

  const validSections = useMemo(() => new Set(NAV_ITEMS.map((item) => item.key)), []);
  const initialSection = useMemo(() => {
    const requested = searchParams.get('section');
    return requested && validSections.has(requested) ? requested : 'chat';
  }, [searchParams, validSections]);
  const activeSection = initialSection;
  const requestedAgentId = searchParams.get('agentId');
  const requestedAgentTab = searchParams.get('agentTab');
  const locationState = location.state as { chatLaunchContext?: ChatLaunchContext } | null;

  // Sidebar collapsed state: chat defaults to collapsed, others to expanded
  const STORAGE_KEY = 'sidebar-collapsed';
  const getDefaultCollapsed = (section: string) => section === 'chat';
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored !== null ? stored === 'true' : getDefaultCollapsed(initialSection);
  });

  const handleToggleSidebar = () => {
    setSidebarCollapsed(prev => {
      localStorage.setItem(STORAGE_KEY, String(!prev));
      return !prev;
    });
  };

  const handleSectionChange = useCallback((section: string) => {
    if (!teamId) return;
    const nextParams = new URLSearchParams();
    nextParams.set('section', section);
    if (section === 'chat' && requestedAgentId) {
      nextParams.set('agentId', requestedAgentId);
    }
    if (section !== 'chat') {
      setChatAgent(null);
    }
    navigate(
      {
        pathname: `/teams/${teamId}`,
        search: `?${nextParams.toString()}`,
      },
      {
        replace: true,
        state: section === 'chat' ? location.state : null,
      },
    );
    // If no explicit user preference stored, apply section default
    if (localStorage.getItem(STORAGE_KEY) === null) {
      setSidebarCollapsed(getDefaultCollapsed(section));
    }
  }, [location.state, navigate, requestedAgentId, teamId]);

  const loadTeam = async () => {
    if (!teamId) return;
    try {
      setLoading(true);
      const response = await apiClient.getTeam(teamId);
      setTeam(response.team);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadTeam();
  }, [teamId]);

  useEffect(() => {
    if (!requestedAgentId) {
      setChatAgent(null);
      return;
    }

    if (!teamId) return;
    let cancelled = false;

    const loadRequestedAgent = async () => {
      try {
        const agent = await agentApi.getAgent(requestedAgentId);
        if (!cancelled) {
          setChatAgent(agent);
        }
      } catch {
        if (!cancelled) {
          setChatAgent(null);
        }
      }
    };

    loadRequestedAgent();

    return () => {
      cancelled = true;
    };
  }, [teamId, requestedAgentId]);

  const canManage = team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin';

  if (loading) {
    return (
      <AppShell className="team-font-cap">
        <div className="space-y-4">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      </AppShell>
    );
  }

  if (error || !team) {
    return (
      <AppShell className="team-font-cap">
        <div className="flex flex-col items-center justify-center py-12 gap-4">
          <p className="text-[hsl(var(--destructive))]">{error || t('teams.notFound')}</p>
          <Link to="/teams">
            <Button variant="outline">{t('teams.backToList')}</Button>
          </Link>
        </div>
      </AppShell>
    );
  }

  const renderContent = () => {
    switch (activeSection) {
      case 'chat':
        return (
          <ChatPanel
            teamId={team.id}
            initialAgent={chatAgent}
            launchContext={locationState?.chatLaunchContext || null}
          />
        );
      case 'agent':
        return (
          <AgentSection
            teamId={team.id}
            onOpenChat={(agent) => { setChatAgent(agent); handleSectionChange('chat'); }}
            onOpenDigitalAvatar={() => handleSectionChange('digital-avatar')}
            initialTab={
              requestedAgentTab === 'missions' || requestedAgentTab === 'task-queue'
                ? requestedAgentTab
                : 'agent-manage'
            }
          />
        );
      case 'documents':
        return <DocumentsTab teamId={team.id} canManage={canManage} />;
      case 'toolkit':
        return <ToolkitSection teamId={team.id} canManage={canManage} />;
      case 'smart-log':
        return <SmartLogTab teamId={team.id} />;
      case 'laboratory':
        return <LaboratorySection teamId={team.id} canManage={canManage} />;
      case 'digital-avatar':
        return <DigitalAvatarSection teamId={team.id} canManage={canManage} />;
      case 'external-users':
        return canManage
          ? <ExternalUsersTab teamId={team.id} />
          : (
            <ChatPanel
              teamId={team.id}
              initialAgent={chatAgent}
              launchContext={locationState?.chatLaunchContext || null}
            />
          );
      case 'team-admin':
        return (
          <TeamAdminSection
            team={team}
            userRole={(team.currentUserRole || 'member') as TeamRole}
            canManage={canManage}
            onUpdate={loadTeam}
            onDelete={() => navigate('/teams')}
          />
        );
      default:
        return (
          <ChatPanel
            teamId={team.id}
            initialAgent={chatAgent}
            launchContext={locationState?.chatLaunchContext || null}
          />
        );
    }
  };

  return (
    <TeamProvider
      value={{
        team,
        canManage,
        activeSection,
        onSectionChange: handleSectionChange,
        onInviteClick: () => setInviteDialogOpen(true),
        sidebarCollapsed,
        onToggleSidebar: handleToggleSidebar,
      }}
    >
      <AppShell className="team-font-cap">
        <Suspense fallback={<SectionLoadingFallback />}>
          {renderContent()}
        </Suspense>

        <CreateInviteDialog
          open={inviteDialogOpen}
          onOpenChange={setInviteDialogOpen}
          teamId={team.id}
          onCreated={() => {
            if (activeSection === 'team-admin') loadTeam();
          }}
        />
      </AppShell>
    </TeamProvider>
  );
}
