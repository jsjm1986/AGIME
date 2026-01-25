import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Link } from 'react-router-dom';
import { Plus, ArrowLeft } from 'lucide-react';
import { Button } from '../components/ui/button';
import { TeamCard } from '../components/team/TeamCard';
import { CreateTeamDialog } from '../components/team/CreateTeamDialog';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { apiClient } from '../api/client';
import type { Team } from '../api/types';

export function TeamsPage() {
  const { t } = useTranslation();
  const [teams, setTeams] = useState<Team[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [createDialogOpen, setCreateDialogOpen] = useState(false);

  const loadTeams = async () => {
    try {
      setLoading(true);
      const response = await apiClient.getTeams();
      setTeams(response.teams);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadTeams();
  }, []);

  return (
    <div className="min-h-screen bg-[hsl(var(--background))]">
      <header className="border-b border-[hsl(var(--border))] bg-[hsl(var(--card))]">
        <div className="container mx-auto px-4 py-4 flex items-center justify-between">
          <div className="flex items-center gap-4">
            <Link to="/dashboard">
              <Button variant="ghost" size="sm">
                <ArrowLeft className="h-4 w-4 mr-2" />
                {t('common.back')}
              </Button>
            </Link>
            <h1 className="text-xl font-semibold">{t('teams.title')}</h1>
          </div>
          <div className="flex items-center gap-2">
            <LanguageSwitcher />
            <Button onClick={() => setCreateDialogOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              {t('teams.createTeam')}
            </Button>
          </div>
        </div>
      </header>

      <main className="container mx-auto px-4 py-8">
        {loading ? (
          <div className="text-center py-12">
            <p className="text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>
          </div>
        ) : error ? (
          <div className="text-center py-12">
            <p className="text-[hsl(var(--destructive))]">{error}</p>
            <Button variant="outline" onClick={loadTeams} className="mt-4">
              {t('common.retry')}
            </Button>
          </div>
        ) : teams.length === 0 ? (
          <div className="text-center py-12">
            <p className="text-[hsl(var(--muted-foreground))] mb-4">{t('teams.noTeams')}</p>
            <Button onClick={() => setCreateDialogOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              {t('teams.createFirstTeam')}
            </Button>
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {teams.map((team) => (
              <TeamCard key={team.id} team={team} />
            ))}
          </div>
        )}
      </main>

      <CreateTeamDialog
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={loadTeams}
      />
    </div>
  );
}
