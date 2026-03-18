import { useCallback, useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, Plus } from 'lucide-react';
import { AppShell } from '../components/layout/AppShell';
import { TeamProvider } from '../contexts/TeamContext';
import { Button } from '../components/ui/button';
import { Skeleton } from '../components/ui/skeleton';
import { apiClient } from '../api/client';
import type { TeamWithStats } from '../api/types';
import { McpRegistryWorkspace } from '../components/team/McpRegistryWorkspace';
import { CreateInviteDialog } from '../components/team/CreateInviteDialog';

export default function McpRegistryPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { teamId } = useParams<{ teamId: string }>();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [inviteDialogOpen, setInviteDialogOpen] = useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => {
    try {
      return window.localStorage.getItem('sidebar-collapsed') === 'true';
    } catch {
      return false;
    }
  });

  const canManage = Boolean(team?.currentUserRole === 'owner' || team?.currentUserRole === 'admin');

  const handleSectionChange = useCallback((section: string) => {
    if (!teamId) return;
    navigate(`/teams/${teamId}?section=${section}`);
  }, [navigate, teamId]);

  const handleToggleSidebar = useCallback(() => {
    setSidebarCollapsed((prev) => {
      try {
        window.localStorage.setItem('sidebar-collapsed', String(!prev));
      } catch {
        // ignore storage failure
      }
      return !prev;
    });
  }, []);

  const handleScrollToInstall = useCallback(() => {
    document.getElementById('mcp-chat-zone')?.scrollIntoView({
      behavior: 'smooth',
      block: 'start',
    });
  }, []);

  const loadTeam = useCallback(async () => {
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
  }, [t, teamId]);

  useEffect(() => {
    void loadTeam();
  }, [loadTeam]);

  if (loading) {
    return (
      <AppShell className="team-font-cap">
        <div className="space-y-4">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-[520px] w-full" />
        </div>
      </AppShell>
    );
  }

  if (error || !team || !teamId) {
    return (
      <AppShell className="team-font-cap">
        <div className="flex flex-col items-center justify-center py-12 gap-4">
          <p className="text-[hsl(var(--destructive))]">{error || t('teams.notFound')}</p>
          <Button variant="outline" onClick={() => navigate('/teams')}>
            {t('teams.backToList')}
          </Button>
        </div>
      </AppShell>
    );
  }

  return (
    <TeamProvider
      value={{
        team,
        canManage,
        activeSection: 'toolkit',
        onSectionChange: handleSectionChange,
        onInviteClick: () => setInviteDialogOpen(true),
        sidebarCollapsed,
        onToggleSidebar: handleToggleSidebar,
      }}
    >
      <AppShell className="team-font-cap">
        <div className="space-y-5">
          <section className="ui-section-panel px-6 py-5">
            <div className="flex flex-col gap-5 lg:flex-row lg:items-start lg:justify-between">
              <div className="space-y-2">
                <div className="ui-kicker">{t('teams.tabs.extensions')}</div>
                <h1 className="ui-heading text-[30px] leading-none">
                  {t('teams.resource.mcpWorkspace.title')}
                </h1>
                <p className="max-w-3xl ui-secondary-text text-sm leading-6">
                  {t(
                    'teams.resource.mcpWorkspace.pageDescription',
                    t('teams.resource.mcpWorkspace.description'),
                  )}
                </p>
              </div>
              <div className="flex items-center gap-2 flex-wrap">
                <Button variant="outline" onClick={() => navigate(`/teams/${teamId}?section=toolkit`)}>
                  <ArrowLeft className="mr-2 h-4 w-4" />
                  {t('teams.resource.mcpWorkspace.backToExtensions')}
                </Button>
                {canManage ? (
                  <Button
                    className="border-[hsl(var(--semantic-extension))]/24 bg-[hsl(var(--semantic-extension))]/10 text-[hsl(var(--semantic-extension))] shadow-none hover:bg-[hsl(var(--semantic-extension))]/16"
                    onClick={handleScrollToInstall}
                  >
                    <Plus className="mr-2 h-4 w-4" />
                    {t('teams.resource.mcpWorkspace.installAction')}
                  </Button>
                ) : null}
              </div>
            </div>
          </section>

          <McpRegistryWorkspace teamId={teamId} canManage={canManage} />
        </div>

        <CreateInviteDialog
          open={inviteDialogOpen}
          onOpenChange={setInviteDialogOpen}
          teamId={team.id}
          onCreated={() => {}}
        />
      </AppShell>
    </TeamProvider>
  );
}
