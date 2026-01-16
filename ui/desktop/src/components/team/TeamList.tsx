import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Users, MoreVertical, Trash2, UserPlus } from 'lucide-react';
import { Team } from './types';
import { listTeams, createTeam, deleteTeam } from './api';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import ServiceStatusIndicator from './ServiceStatusIndicator';
import { JoinTeamDialog } from './invites';
import { getTeamConnectionMode } from './api';

interface TeamListProps {
  onSelectTeam: (team: Team) => void;
  selectedTeamId: string | null;
}

const TeamList: React.FC<TeamListProps> = ({ onSelectTeam, selectedTeamId }) => {
  const { t } = useTranslation('team');
  const [teams, setTeams] = useState<Team[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [newTeamName, setNewTeamName] = useState('');
  const [newTeamDescription, setNewTeamDescription] = useState('');
  const [isCreating, setIsCreating] = useState(false);

  // Join team state
  const [showJoinDialog, setShowJoinDialog] = useState(false);
  const [selectedTeamToJoin, setSelectedTeamToJoin] = useState<Team | null>(null);
  const isRemoteLAN = getTeamConnectionMode() === 'lan';

  const loadTeams = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const response = await listTeams();
      setTeams(response.teams);
    } catch (err) {
      console.error('Failed to load teams:', err);
      setError(t('loadTeamsError', 'Failed to load teams'));
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    loadTeams();
  }, [loadTeams]);

  const handleCreateTeam = async () => {
    if (!newTeamName.trim()) return;

    setIsCreating(true);
    try {
      const team = await createTeam({
        name: newTeamName.trim(),
        description: newTeamDescription.trim() || undefined,
      });
      setTeams((prev) => [team, ...prev]);
      setShowCreateDialog(false);
      setNewTeamName('');
      setNewTeamDescription('');
    } catch (err) {
      console.error('Failed to create team:', err);
      setError(t('createTeamError', 'Failed to create team'));
    } finally {
      setIsCreating(false);
    }
  };

  const handleDeleteTeam = async (teamId: string) => {
    if (!confirm(t('deleteTeamConfirm', 'Are you sure you want to delete this team?'))) {
      return;
    }

    try {
      await deleteTeam(teamId);
      setTeams((prev) => prev.filter((t) => t.id !== teamId));
    } catch (err) {
      console.error('Failed to delete team:', err);
      setError(t('deleteTeamError', 'Failed to delete team'));
    }
  };

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleDateString();
  };

  if (isLoading && teams.length === 0) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center justify-between p-4 border-b border-border-subtle">
        <div className="flex items-center gap-3">
          <h1 className="text-xl font-semibold text-text-default">
            {t('title', 'Teams')}
          </h1>
          <ServiceStatusIndicator showLabel={true} />
        </div>
        <Button
          onClick={() => setShowCreateDialog(true)}
          className="flex items-center gap-2"
        >
          <Plus size={16} />
          {t('createTeam', 'Create Team')}
        </Button>
      </div>

      {/* Error message */}
      {error && (
        <div className="mx-4 mt-4 p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400 text-sm">
          {error}
          <button
            onClick={() => setError(null)}
            className="ml-2 text-red-500 hover:text-red-700"
          >
            ×
          </button>
        </div>
      )}

      {/* Team list */}
      <div className="flex-1 overflow-y-auto p-4">
        {teams.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-64 text-text-muted">
            <Users size={48} className="mb-4 opacity-50" />
            <p className="text-lg">{t('noTeams', 'No teams yet')}</p>
            <p className="text-sm mt-2">
              {t('noTeamsDescription', 'Create a team to start collaborating')}
            </p>
          </div>
        ) : (
          <div className="grid gap-4">
            {teams.map((team) => (
              <div
                key={team.id}
                onClick={() => onSelectTeam(team)}
                className={`
                  p-4 rounded-lg border cursor-pointer transition-all
                  ${selectedTeamId === team.id
                    ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20'
                    : 'border-border-subtle hover:border-border-default hover:bg-background-muted'
                  }
                `}
              >
                <div className="flex items-start justify-between">
                  <div className="flex-1">
                    <h3 className="font-medium text-text-default">{team.name}</h3>
                    {team.description && (
                      <p className="text-sm text-text-muted mt-1 line-clamp-2">
                        {team.description}
                      </p>
                    )}
                    <p className="text-xs text-text-muted mt-2">
                      <span className="text-xs text-text-muted">{t('created', 'Created')} {formatDate(team.createdAt)}</span>
                    </p>
                  </div>
                  {/* Join button for remote LAN teams */}
                  {isRemoteLAN && (
                    <Button
                      onClick={(e) => {
                        e.stopPropagation();
                        setSelectedTeamToJoin(team);
                        setShowJoinDialog(true);
                      }}
                      variant="outline"
                      size="sm"
                      className="mt-2 w-full"
                    >
                      <UserPlus size={14} className="mr-1" />
                      {t('team.join', '加入团队')}
                    </Button>
                  )}
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button
                        onClick={(e) => e.stopPropagation()}
                        className="p-1 rounded hover:bg-background-muted"
                      >
                        <MoreVertical size={16} className="text-text-muted" />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDeleteTeam(team.id);
                        }}
                        className="text-red-600 dark:text-red-400"
                      >
                        <Trash2 size={14} className="mr-2" />
                        {t('delete', 'Delete')}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Create team dialog */}
      {showCreateDialog && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background-default rounded-lg shadow-xl p-6 w-full max-w-md mx-4">
            <h2 className="text-lg font-semibold text-text-default mb-4">
              {t('createTeam', 'Create Team')}
            </h2>
            <div className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-text-default mb-1">
                  {t('teamName', 'Team Name')} *
                </label>
                <input
                  type="text"
                  value={newTeamName}
                  onChange={(e) => setNewTeamName(e.target.value)}
                  placeholder={t('teamNamePlaceholder', 'Enter team name')}
                  className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500"
                  autoFocus
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-text-default mb-1">
                  {t('description', 'Description')}
                </label>
                <textarea
                  value={newTeamDescription}
                  onChange={(e) => setNewTeamDescription(e.target.value)}
                  placeholder={t('descriptionPlaceholder', 'Optional description')}
                  rows={3}
                  className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 resize-none"
                />
              </div>
            </div>
            <div className="flex justify-end gap-3 mt-6">
              <Button
                variant="outline"
                onClick={() => {
                  setShowCreateDialog(false);
                  setNewTeamName('');
                  setNewTeamDescription('');
                }}
                disabled={isCreating}
              >
                {t('cancel', 'Cancel')}
              </Button>
              <Button
                onClick={handleCreateTeam}
                disabled={!newTeamName.trim() || isCreating}
              >
                {isCreating ? t('creating', 'Creating...') : t('create', 'Create')}
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Join Team Dialog */}
      <JoinTeamDialog
        open={showJoinDialog}
        onClose={() => {
          setShowJoinDialog(false);
          setSelectedTeamToJoin(null);
        }}
        teamId={selectedTeamToJoin?.id}
        teamName={selectedTeamToJoin?.name}
        onSuccess={() => {
          setShowJoinDialog(false);
          setSelectedTeamToJoin(null);
          loadTeams(); // Reload teams after joining
        }}
      />
    </div>
  );
};

export default TeamList;
