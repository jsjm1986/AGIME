import React, { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, UserPlus } from 'lucide-react';
import { CloudServer, Team, TeamSummary } from './types';
import { Button } from '../ui/button';
import { CloudServerList, AddServerDialog, getActiveServer, setActiveServer } from './servers';
import { JoinTeamDialog } from './invites';
import TeamList from './TeamList';
import TeamDetail from './TeamDetail';
import { getTeam } from './api';

interface CloudTeamViewProps {
    onBack?: () => void;
}

type ViewState = 'servers' | 'teams' | 'detail';

const CloudTeamView: React.FC<CloudTeamViewProps> = ({ onBack }) => {
    const { t } = useTranslation('team');

    const [viewState, setViewState] = useState<ViewState>('servers');
    const [selectedServer, setSelectedServer] = useState<CloudServer | null>(null);
    const [selectedTeam, setSelectedTeam] = useState<TeamSummary | null>(null);
    const [showAddServer, setShowAddServer] = useState(false);
    const [showJoinTeam, setShowJoinTeam] = useState(false);
    const [isLoadingTeam, setIsLoadingTeam] = useState(false);
    const [error, setError] = useState<string | null>(null);

    // Check for active server on mount
    useEffect(() => {
        const activeServer = getActiveServer();
        if (activeServer && activeServer.status === 'online') {
            setSelectedServer(activeServer);
            setViewState('teams');
        }
    }, []);

    const handleSelectServer = useCallback((server: CloudServer) => {
        setActiveServer(server.id);
        setSelectedServer(server);
        setViewState('teams');
    }, []);

    const handleAddServerSuccess = useCallback((server: CloudServer) => {
        setShowAddServer(false);
        handleSelectServer(server);
    }, [handleSelectServer]);

    const handleSelectTeam = useCallback(async (team: Team) => {
        setIsLoadingTeam(true);
        setError(null);
        try {
            const teamSummary = await getTeam(team.id);
            setSelectedTeam(teamSummary);
            setViewState('detail');
        } catch (err) {
            console.error('Failed to load team:', err);
            setError(t('teamDetailError', 'Failed to load team details'));
        } finally {
            setIsLoadingTeam(false);
        }
    }, [t]);

    const handleBackToServers = useCallback(() => {
        setSelectedServer(null);
        setViewState('servers');
    }, []);

    const handleBackToTeams = useCallback(() => {
        setSelectedTeam(null);
        setViewState('teams');
    }, []);

    const handleTeamDeleted = useCallback((teamId: string) => {
        if (selectedTeam?.team.id === teamId) {
            handleBackToTeams();
        }
    }, [selectedTeam, handleBackToTeams]);

    const handleJoinSuccess = useCallback((_teamId: string) => {
        setShowJoinTeam(false);
        // Refresh the team list
        if (selectedServer) {
            setViewState('teams');
        }
    }, [selectedServer]);

    // Render team detail
    if (viewState === 'detail' && selectedTeam) {
        return (
            <TeamDetail
                teamSummary={selectedTeam}
                isLoading={isLoadingTeam}
                error={error}
                onBack={handleBackToTeams}
                onRetry={() => selectedTeam && handleSelectTeam(selectedTeam.team)}
                onTeamDeleted={handleTeamDeleted}
                currentUserId={selectedTeam.currentUserId}
            />
        );
    }

    // Render team list for selected server
    if (viewState === 'teams' && selectedServer) {
        return (
            <div className="flex flex-col h-full">
                {/* Header with server info */}
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <button
                        onClick={handleBackToServers}
                        className="p-2 rounded-lg hover:bg-background-muted"
                    >
                        <ArrowLeft size={20} className="text-text-muted" />
                    </button>
                    <div className="flex-1">
                        <h2 className="font-medium text-text-default">{selectedServer.name}</h2>
                        <p className="text-xs text-text-muted">
                            {selectedServer.userEmail || selectedServer.url}
                        </p>
                    </div>
                    <Button
                        variant="outline"
                        size="sm"
                        onClick={() => setShowJoinTeam(true)}
                        className="flex items-center gap-2"
                    >
                        <UserPlus size={14} />
                        {t('join.button', 'Join Team')}
                    </Button>
                </div>

                {/* Team list */}
                <div className="flex-1 overflow-hidden">
                    <TeamList
                        onSelectTeam={handleSelectTeam}
                        selectedTeamId={selectedTeam?.team.id ?? null}
                    />
                </div>

                {/* Join team dialog */}
                <JoinTeamDialog
                    open={showJoinTeam}
                    onClose={() => setShowJoinTeam(false)}
                    onSuccess={handleJoinSuccess}
                />
            </div>
        );
    }

    // Render server list
    return (
        <div className="flex flex-col h-full">
            {/* Back button if onBack provided */}
            {onBack && (
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <button
                        onClick={onBack}
                        className="p-2 rounded-lg hover:bg-background-muted"
                    >
                        <ArrowLeft size={20} className="text-text-muted" />
                    </button>
                    <h2 className="font-medium text-text-default">
                        {t('cloud.title', 'Cloud Servers')}
                    </h2>
                </div>
            )}

            {/* Server list */}
            <div className="flex-1 overflow-hidden">
                <CloudServerList
                    onSelectServer={handleSelectServer}
                    onAddServer={() => setShowAddServer(true)}
                />
            </div>

            {/* Add server dialog */}
            <AddServerDialog
                open={showAddServer}
                onClose={() => setShowAddServer(false)}
                onSuccess={handleAddServerSuccess}
            />
        </div>
    );
};

export default CloudTeamView;
