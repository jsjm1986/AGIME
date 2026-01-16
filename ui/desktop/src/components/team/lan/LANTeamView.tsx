import React, { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, Settings } from 'lucide-react';
import { LANConnection, TeamSummary, Team } from '../types';
import { Button } from '../../ui/button';
import LANDeviceList from './LANDeviceList';
import ConnectLANDialog from './ConnectLANDialog';
import LANShareSettings from './LANShareSettings';
import TeamList from '../TeamList';
import TeamDetail from '../TeamDetail';
import { getTeam } from '../api';

interface LANTeamViewProps {
    onBack?: () => void;
}

type ViewState = 'connections' | 'settings' | 'local-teams' | 'teams' | 'detail';

const LANTeamView: React.FC<LANTeamViewProps> = ({ onBack }) => {
    const { t } = useTranslation('team');

    const [viewState, setViewState] = useState<ViewState>('connections');
    const [selectedConnection, setSelectedConnection] = useState<LANConnection | null>(null);
    const [selectedTeam, setSelectedTeam] = useState<TeamSummary | null>(null);
    const [showConnectDialog, setShowConnectDialog] = useState(false);
    const [isLoadingTeam, setIsLoadingTeam] = useState(false);
    const [error, setError] = useState<string | null>(null);

    // Handle selecting local teams (no connection)
    const handleSelectLocalTeams = useCallback(() => {
        setSelectedConnection(null);
        setViewState('local-teams');

        // Clear connection mode - API will use local agimed
        try {
            localStorage.removeItem('AGIME_TEAM_ACTIVE_LAN_CONNECTION');
            localStorage.removeItem('AGIME_TEAM_CONNECTION_MODE');
            localStorage.removeItem('AGIME_TEAM_LAN_SERVER_URL');
            localStorage.removeItem('AGIME_TEAM_LAN_SECRET_KEY');
        } catch {
            // Ignore storage errors
        }
    }, []);

    const handleSelectConnection = useCallback((connection: LANConnection) => {
        setSelectedConnection(connection);
        setViewState('teams');

        // Set the connection as active for API calls
        // Must use keys that match api.ts STORAGE_KEYS
        try {
            localStorage.setItem('AGIME_TEAM_ACTIVE_LAN_CONNECTION', connection.id);
            // Set remote server config for API calls - must match STORAGE_KEYS in api.ts
            localStorage.setItem('AGIME_TEAM_CONNECTION_MODE', 'lan');
            localStorage.setItem('AGIME_TEAM_LAN_SERVER_URL', `http://${connection.host}:${connection.port}/api/team`);
            localStorage.setItem('AGIME_TEAM_LAN_SECRET_KEY', connection.secretKey);
        } catch {
            // Ignore storage errors
        }
    }, []);

    const handleConnectionSuccess = useCallback((connection: LANConnection) => {
        setShowConnectDialog(false);
        // Optionally select the new connection immediately
        handleSelectConnection(connection);
    }, [handleSelectConnection]);

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

    const handleBackToConnections = useCallback(() => {
        setSelectedConnection(null);
        setViewState('connections');

        // Clear active connection - must match STORAGE_KEYS in api.ts
        try {
            localStorage.removeItem('AGIME_TEAM_ACTIVE_LAN_CONNECTION');
            localStorage.removeItem('AGIME_TEAM_CONNECTION_MODE');
            localStorage.removeItem('AGIME_TEAM_LAN_SERVER_URL');
            localStorage.removeItem('AGIME_TEAM_LAN_SECRET_KEY');
        } catch {
            // Ignore
        }
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

    // Render team list for selected connection
    if (viewState === 'teams' && selectedConnection) {
        return (
            <div className="flex flex-col h-full">
                {/* Header with connection info */}
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <button
                        onClick={handleBackToConnections}
                        className="p-2 rounded-lg hover:bg-background-muted"
                    >
                        <ArrowLeft size={20} className="text-text-muted" />
                    </button>
                    <div className="flex-1">
                        <h2 className="font-medium text-text-default">{selectedConnection.name}</h2>
                        <p className="text-xs text-text-muted">
                            {selectedConnection.host}:{selectedConnection.port}
                        </p>
                    </div>
                </div>

                {/* Team list */}
                <div className="flex-1 overflow-hidden">
                    <TeamList
                        onSelectTeam={handleSelectTeam}
                        selectedTeamId={selectedTeam?.team.id ?? null}
                    />
                </div>
            </div>
        );
    }

    // Render local teams (no connection required)
    if (viewState === 'local-teams') {
        return (
            <div className="flex flex-col h-full">
                {/* Header */}
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <button
                        onClick={() => setViewState('connections')}
                        className="p-2 rounded-lg hover:bg-background-muted"
                    >
                        <ArrowLeft size={20} className="text-text-muted" />
                    </button>
                    <h2 className="font-medium text-text-default">
                        {t('lan.localTeamsTitle', 'My Local Teams')}
                    </h2>
                </div>

                {/* Local team list */}
                <div className="flex-1 overflow-hidden">
                    <TeamList
                        onSelectTeam={handleSelectTeam}
                        selectedTeamId={selectedTeam?.team.id ?? null}
                    />
                </div>
            </div>
        );
    }

    // Render settings

    if (viewState === 'settings') {
        return (
            <div className="flex flex-col h-full">
                {/* Header */}
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <button
                        onClick={() => setViewState('connections')}
                        className="p-2 rounded-lg hover:bg-background-muted"
                    >
                        <ArrowLeft size={20} className="text-text-muted" />
                    </button>
                    <h2 className="font-medium text-text-default">
                        {t('lan.settingsTitle', 'LAN Sharing Settings')}
                    </h2>
                </div>

                {/* Settings content */}
                <div className="flex-1 overflow-y-auto p-4">
                    <LANShareSettings />
                </div>
            </div>
        );
    }

    // Render connections list
    return (
        <div className="flex flex-col h-full">
            {/* Back button and settings */}
            <div className="flex items-center justify-between p-4 border-b border-border-subtle">
                <div className="flex items-center gap-3">
                    {onBack && (
                        <button
                            onClick={onBack}
                            className="p-2 rounded-lg hover:bg-background-muted"
                        >
                            <ArrowLeft size={20} className="text-text-muted" />
                        </button>
                    )}
                    <h2 className="font-medium text-text-default">
                        {t('lan.mode', 'LAN Mode')}
                    </h2>
                </div>
                <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setViewState('settings')}
                    className="flex items-center gap-2"
                >
                    <Settings size={16} />
                    {t('lan.settings', 'Settings')}
                </Button>
            </div>

            {/* Connection list */}
            <div className="flex-1 overflow-hidden">
                <LANDeviceList
                    onSelectConnection={handleSelectConnection}
                    onAddConnection={() => setShowConnectDialog(true)}
                    onSelectLocalTeams={handleSelectLocalTeams}
                />
            </div>

            {/* Connect dialog */}
            <ConnectLANDialog
                open={showConnectDialog}
                onClose={() => setShowConnectDialog(false)}
                onSuccess={handleConnectionSuccess}
            />
        </div>
    );
};

export default LANTeamView;
