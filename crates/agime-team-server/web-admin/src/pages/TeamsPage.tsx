import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { TeamCard } from '../components/team/TeamCard';
import { CreateTeamDialog } from '../components/team/CreateTeamDialog';
import { AppShell } from '../components/layout/AppShell';
import { PageHeader } from '../components/layout/PageHeader';
import { SearchInput } from '../components/ui/search-input';
import { SkeletonCard } from '../components/ui/skeleton';
import { apiClient } from '../api/client';
import type { Team } from '../api/types';

type SortOption = 'name' | 'created';

export function TeamsPage() {
  const { t } = useTranslation();
  const [teams, setTeams] = useState<Team[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [search, setSearch] = useState('');
  const [sortBy, setSortBy] = useState<SortOption>('created');

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

  const filteredTeams = useMemo(() => {
    let result = [...teams];

    if (search) {
      const query = search.toLowerCase();
      result = result.filter(
        team =>
          team.name.toLowerCase().includes(query) ||
          team.description?.toLowerCase().includes(query)
      );
    }

    result.sort((a, b) => {
      switch (sortBy) {
        case 'name':
          return a.name.localeCompare(b.name);
        case 'created':
        default:
          return new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime();
      }
    });

    return result;
  }, [teams, search, sortBy]);

  const PlusIcon = () => (
    <svg className="w-4 h-4 mr-2" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
    </svg>
  );

  return (
    <AppShell>
      <PageHeader
        title={t('teams.title')}
        actions={
          <Button onClick={() => setCreateDialogOpen(true)}>
            <PlusIcon />
            {t('teams.createTeam')}
          </Button>
        }
      />

      {/* Search and Sort */}
      <div className="flex flex-col sm:flex-row gap-4 mb-6">
        <div className="flex-1">
          <SearchInput
            placeholder={t('teams.searchPlaceholder')}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onClear={() => setSearch('')}
          />
        </div>
        <select
          value={sortBy}
          onChange={(e) => setSortBy(e.target.value as SortOption)}
          className="h-10 px-3 rounded-lg border border-[hsl(var(--input))] bg-[hsl(var(--background))]"
        >
          <option value="created">{t('sort.created')}</option>
          <option value="name">{t('sort.name')}</option>
        </select>
      </div>

      {/* Content */}
      {loading ? (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {[1, 2, 3].map((i) => (
            <SkeletonCard key={i} />
          ))}
        </div>
      ) : error ? (
        <div className="text-center py-12">
          <p className="text-[hsl(var(--destructive))]">{error}</p>
          <Button variant="outline" onClick={loadTeams} className="mt-4">
            {t('common.retry')}
          </Button>
        </div>
      ) : filteredTeams.length === 0 ? (
        <div className="text-center py-12">
          <p className="text-[hsl(var(--muted-foreground))] mb-4">
            {search ? t('teams.noSearchResults') : t('teams.noTeams')}
          </p>
          {!search && (
            <Button onClick={() => setCreateDialogOpen(true)}>
              <PlusIcon />
              {t('teams.createFirstTeam')}
            </Button>
          )}
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {filteredTeams.map((team) => (
            <TeamCard key={team.id} team={team} />
          ))}
        </div>
      )}

      <CreateTeamDialog
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={loadTeams}
      />
    </AppShell>
  );
}
