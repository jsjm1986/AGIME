import { useState, useEffect } from 'react';
import { useParams, useNavigate, useSearchParams, Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { AppShell } from '../components/layout/AppShell';
import { Skeleton } from '../components/ui/skeleton';
import { TeamProvider } from '../contexts/TeamContext';
import { DocumentsTab } from '../components/team/DocumentsTab';
import { SmartLogTab } from '../components/team/SmartLogTab';
import { ChatPanel } from '../components/team/ChatPanel';
import { ToolkitSection } from '../components/team/ToolkitSection';
import { AgentSection } from '../components/team/AgentSection';
import { TeamAdminSection } from '../components/team/TeamAdminSection';
import { LaboratorySection } from '../components/team/LaboratorySection';
import { CreateInviteDialog } from '../components/team/CreateInviteDialog';
import { apiClient } from '../api/client';
import type { TeamWithStats, TeamRole } from '../api/types';
import type { TeamAgent } from '../api/agent';

export function TeamDetailPage() {
  const { t } = useTranslation();
  const { teamId } = useParams<{ teamId: string }>();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [inviteDialogOpen, setInviteDialogOpen] = useState(false);
  const [chatAgent, setChatAgent] = useState<TeamAgent | null>(null);

  // Read initial section from URL search param, default to 'chat'
  const initialSection = searchParams.get('section') || 'chat';
  const [activeSection, setActiveSection] = useState(initialSection);

  // Sync activeSection when browser back/forward changes the URL
  useEffect(() => {
    const urlSection = searchParams.get('section');
    if (urlSection && urlSection !== activeSection) {
      setActiveSection(urlSection);
    }
  }, [searchParams, activeSection]);

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

  const handleSectionChange = (section: string) => {
    setActiveSection(section);
    setSearchParams({ section }, { replace: true });
    // If no explicit user preference stored, apply section default
    if (localStorage.getItem(STORAGE_KEY) === null) {
      setSidebarCollapsed(getDefaultCollapsed(section));
    }
  };

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

  const canManage = team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin';

  if (loading) {
    return (
      <AppShell>
        <div className="space-y-4">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-64 w-full" />
        </div>
      </AppShell>
    );
  }

  if (error || !team) {
    return (
      <AppShell>
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
        return <ChatPanel teamId={team.id} initialAgent={chatAgent} />;
      case 'agent':
        return (
          <AgentSection
            teamId={team.id}
            onOpenChat={(agent) => { setChatAgent(agent); handleSectionChange('chat'); }}
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
        return <ChatPanel teamId={team.id} initialAgent={chatAgent} />;
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
      <AppShell>
        {renderContent()}

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
