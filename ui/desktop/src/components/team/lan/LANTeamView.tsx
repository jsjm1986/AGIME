import React, { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, Settings, Plus, Monitor, Wifi } from 'lucide-react';
import { TeamSummary, Team } from '../types';
import type { DataSource } from '../sources/types';
import { Button } from '../../ui/button';
import ConnectLANDialog from './ConnectLANDialog';
import LANShareSettings from './LANShareSettings';
import LANDeviceCard from './LANDeviceCard';
import TeamList from '../TeamList';
import TeamDetail from '../TeamDetail';
import { getTeam } from '../api';
import { sourceManager } from '../sources/sourceManager';

interface LANTeamViewProps {
    onBack?: () => void;
}

type ViewState = 'connections' | 'settings' | 'local-teams' | 'teams' | 'detail';

const LANTeamView: React.FC<LANTeamViewProps> = ({ onBack }) => {
    const { t } = useTranslation('team');

    const [viewState, setViewState] = useState<ViewState>('connections');
    const [selectedSource, setSelectedSource] = useState<DataSource | null>(null);
    const [selectedTeam, setSelectedTeam] = useState<TeamSummary | null>(null);
    const [showConnectDialog, setShowConnectDialog] = useState(false);
    const [isLoadingTeam, setIsLoadingTeam] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [lanSources, setLanSources] = useState<DataSource[]>([]);

    // Load LAN sources from SourceManager
    useEffect(() => {
        const loadSources = () => {
            const sources = sourceManager.getSourcesByType('lan');
            setLanSources(sources);
        };
        loadSources();

        // Listen for source changes
        const handleSourceChange = () => loadSources();
        sourceManager.addListener(handleSourceChange);
        return () => sourceManager.removeListener(handleSourceChange);
    }, []);

    // Handle selecting local teams (no connection)
    const handleSelectLocalTeams = useCallback(() => {
        setSelectedSource(null);
        setViewState('local-teams');
        // Reset to local source via SourceManager
        sourceManager.resetToLocal();
    }, []);

    const handleSelectSource = useCallback((source: DataSource) => {
        setSelectedSource(source);
        setViewState('teams');
        // Set this source as active via SourceManager
        sourceManager.setActiveSource(source.id);
    }, []);

    const handleConnectionSuccess = useCallback((source: DataSource) => {
        setShowConnectDialog(false);
        // Refresh LAN sources list
        setLanSources(sourceManager.getSourcesByType('lan'));
        // Optionally select the new source immediately
        handleSelectSource(source);
    }, [handleSelectSource]);

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
        setSelectedSource(null);
        setViewState('connections');
        // Reset to local source via SourceManager
        sourceManager.resetToLocal();
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

    const handleRemoveSource = useCallback((sourceId: string) => {
        if (confirm(t('lan.removeConfirm', 'Remove this connection?'))) {
            sourceManager.unregisterSource(sourceId);
            // Refresh LAN sources list
            setLanSources(sourceManager.getSourcesByType('lan'));
        }
    }, [t]);

    const handleRefreshSource = useCallback(async (source: DataSource) => {
        const health = await sourceManager.checkHealth(source.id);
        if (health.healthy) {
            sourceManager.updateSourceStatus(source.id, 'online');
        } else {
            sourceManager.updateSourceStatus(source.id, 'error', health.error);
        }
        // Refresh LAN sources list
        setLanSources(sourceManager.getSourcesByType('lan'));
    }, []);

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

    // Render team list for selected source
    if (viewState === 'teams' && selectedSource) {
        return (
            <div className="flex flex-col h-full">
                {/* Header with source info */}
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <button
                        onClick={handleBackToConnections}
                        className="p-2 rounded-lg hover:bg-background-muted"
                    >
                        <ArrowLeft size={20} className="text-text-muted" />
                    </button>
                    <div className="flex-1">
                        <h2 className="font-medium text-text-default">{selectedSource.name}</h2>
                        <p className="text-xs text-text-muted">
                            {selectedSource.connection.url}
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

            {/* Connection list - inline implementation */}
            <div className="flex-1 overflow-y-auto p-4">
                {/* Local teams option */}
                <button
                    onClick={handleSelectLocalTeams}
                    className="w-full p-4 mb-4 rounded-lg border-2 border-border-subtle hover:border-teal-500 hover:bg-teal-50 dark:hover:bg-teal-900/10 transition-all text-left"
                >
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-gray-100 dark:bg-gray-800">
                            <Monitor size={20} className="text-gray-600" />
                        </div>
                        <div>
                            <p className="font-medium">{t('lan.localTeams', 'My Local Teams')}</p>
                            <p className="text-sm text-text-muted">{t('lan.localTeamsDesc', 'Teams stored on this device')}</p>
                        </div>
                    </div>
                </button>

                {/* LAN connections */}
                <div className="space-y-2">
                    <div className="flex items-center justify-between mb-2">
                        <h3 className="text-sm font-medium text-text-muted">{t('lan.connections', 'LAN Connections')}</h3>
                        <button
                            onClick={() => setShowConnectDialog(true)}
                            className="text-xs text-blue-600 hover:text-blue-800 flex items-center gap-1"
                        >
                            <Plus size={14} />
                            {t('lan.addConnection', 'Add')}
                        </button>
                    </div>
                    {lanSources.length === 0 ? (
                        <div className="text-center py-8 text-text-muted">
                            <Wifi size={32} className="mx-auto mb-2 opacity-50" />
                            <p>{t('lan.noConnections', 'No LAN connections')}</p>
                            <p className="text-sm mt-1">{t('lan.noConnectionsHint', 'Connect to other devices on your network')}</p>
                        </div>
                    ) : (
                        lanSources.map(source => (
                            <LANDeviceCard
                                key={source.id}
                                source={source}
                                onSelect={() => handleSelectSource(source)}
                                onRemove={() => handleRemoveSource(source.id)}
                                onRefresh={() => handleRefreshSource(source)}
                            />
                        ))
                    )}
                </div>
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
