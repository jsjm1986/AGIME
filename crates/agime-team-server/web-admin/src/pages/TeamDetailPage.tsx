import { useState, useEffect } from 'react';
import { useParams, useNavigate, Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, UserPlus } from 'lucide-react';
import { Button } from '../components/ui/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../components/ui/tabs';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { MembersTab } from '../components/team/MembersTab';
import { SkillsTab } from '../components/team/SkillsTab';
import { RecipesTab } from '../components/team/RecipesTab';
import { ExtensionsTab } from '../components/team/ExtensionsTab';
import { InvitesTab } from '../components/team/InvitesTab';
import { SettingsTab } from '../components/team/SettingsTab';
import { CreateInviteDialog } from '../components/team/CreateInviteDialog';
import { apiClient } from '../api/client';
import type { TeamWithStats } from '../api/types';

export function TeamDetailPage() {
  const { t } = useTranslation();
  const { teamId } = useParams<{ teamId: string }>();
  const navigate = useNavigate();
  const [team, setTeam] = useState<TeamWithStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [activeTab, setActiveTab] = useState('members');
  const [inviteDialogOpen, setInviteDialogOpen] = useState(false);

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

  const isOwner = team?.currentUserId === team?.ownerId;
  const canManage = isOwner; // 目前只有 owner 可以管理

  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <p className="text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>
      </div>
    );
  }

  if (error || !team) {
    return (
      <div className="min-h-screen flex flex-col items-center justify-center gap-4">
        <p className="text-[hsl(var(--destructive))]">{error || t('teams.notFound')}</p>
        <Link to="/teams">
          <Button variant="outline">{t('teams.backToList')}</Button>
        </Link>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[hsl(var(--background))]">
      <header className="border-b border-[hsl(var(--border))] bg-[hsl(var(--card))]">
        <div className="container mx-auto px-4 py-4">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-4">
              <Link to="/teams">
                <Button variant="ghost" size="sm">
                  <ArrowLeft className="h-4 w-4 mr-2" />
                  {t('common.back')}
                </Button>
              </Link>
              <div>
                <h1 className="text-xl font-semibold">{team.name}</h1>
                {team.description && (
                  <p className="text-sm text-[hsl(var(--muted-foreground))]">{team.description}</p>
                )}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <LanguageSwitcher />
              {canManage && (
                <Button onClick={() => setInviteDialogOpen(true)}>
                  <UserPlus className="h-4 w-4 mr-2" />
                  {t('teams.inviteMember')}
                </Button>
              )}
            </div>
          </div>
        </div>
      </header>

      <main className="container mx-auto px-4 py-6">
        <Tabs value={activeTab} onValueChange={setActiveTab}>
          <TabsList className="mb-6">
            <TabsTrigger value="members">
              {t('teams.tabs.members')} ({team.membersCount})
            </TabsTrigger>
            <TabsTrigger value="skills">
              {t('teams.tabs.skills')} ({team.skillsCount})
            </TabsTrigger>
            <TabsTrigger value="recipes">
              {t('teams.tabs.recipes')} ({team.recipesCount})
            </TabsTrigger>
            <TabsTrigger value="extensions">
              {t('teams.tabs.extensions')} ({team.extensionsCount})
            </TabsTrigger>
            {canManage && (
              <TabsTrigger value="invites">{t('teams.tabs.invites')}</TabsTrigger>
            )}
            {isOwner && (
              <TabsTrigger value="settings">{t('teams.tabs.settings')}</TabsTrigger>
            )}
          </TabsList>

          <TabsContent value="members">
            <MembersTab teamId={team.id} userRole={isOwner ? 'owner' : 'member'} onUpdate={loadTeam} />
          </TabsContent>
          <TabsContent value="skills">
            <SkillsTab teamId={team.id} canManage={canManage} />
          </TabsContent>
          <TabsContent value="recipes">
            <RecipesTab teamId={team.id} canManage={canManage} />
          </TabsContent>
          <TabsContent value="extensions">
            <ExtensionsTab teamId={team.id} canManage={canManage} />
          </TabsContent>
          {canManage && (
            <TabsContent value="invites">
              <InvitesTab teamId={team.id} />
            </TabsContent>
          )}
          {isOwner && (
            <TabsContent value="settings">
              <SettingsTab team={team} onUpdate={loadTeam} onDelete={() => navigate('/teams')} />
            </TabsContent>
          )}
        </Tabs>
      </main>

      <CreateInviteDialog
        open={inviteDialogOpen}
        onOpenChange={setInviteDialogOpen}
        teamId={team.id}
        onCreated={() => {
          if (activeTab === 'invites') loadTeam();
        }}
      />
    </div>
  );
}
