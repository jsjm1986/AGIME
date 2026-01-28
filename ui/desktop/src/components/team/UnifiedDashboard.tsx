import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, Wifi, ArrowRight, Star, Clock, Monitor, Plus, RefreshCw, Sparkles, Book, Puzzle } from 'lucide-react';
import { RecentTeam, getRecentTeams, formatRelativeTime } from './recentTeamsStore';
import { sourceManager } from './sources/sourceManager';
import type { DataSource } from './sources/types';
import { AddSourceDialog } from './dashboard/AddSourceDialog';

interface UnifiedDashboardProps {
    onNavigateCloud: () => void;
    onNavigateLan: () => void;
    onSelectRecentTeam: (team: RecentTeam) => void;
    onSelectSource?: (source: DataSource) => void;
    onNavigateResources?: (type: 'skill' | 'recipe' | 'extension') => void;
}

const UnifiedDashboard: React.FC<UnifiedDashboardProps> = ({
    onNavigateCloud,
    onNavigateLan,
    onSelectRecentTeam,
    onSelectSource,
    onNavigateResources,
}) => {
    const { t } = useTranslation('team');
    const [sources, setSources] = useState<DataSource[]>([]);
    const [loading, setLoading] = useState(true);
    const [showAddDialog, setShowAddDialog] = useState(false);
    const [refreshing, setRefreshing] = useState(false);

    // Initialize sources on mount
    useEffect(() => {
        const init = async () => {
            await sourceManager.initialize();
            setSources(sourceManager.getAllSources());
            setLoading(false);
        };
        init();
    }, []);

    // Get source counts
    const localSources = sources.filter(s => s.type === 'local');
    const cloudSources = sources.filter(s => s.type === 'cloud');
    const lanSources = sources.filter(s => s.type === 'lan');

    const cloudOnlineCount = cloudSources.filter(s => s.status === 'online').length;
    const lanOnlineCount = lanSources.filter(s => s.status === 'online').length;
    const recentTeams = getRecentTeams().slice(0, 5);

    const handleRefreshAll = async () => {
        setRefreshing(true);
        await sourceManager.checkAllHealth();
        setSources(sourceManager.getAllSources());
        setRefreshing(false);
    };

    const handleAddSuccess = () => {
        setSources(sourceManager.getAllSources());
    };

    const getSourceIcon = (type: string) => {
        switch (type) {
            case 'local': return <Monitor size={14} className="text-gray-500" />;
            case 'cloud': return <Cloud size={14} className="text-blue-500" />;
            case 'lan': return <Wifi size={14} className="text-green-500" />;
            default: return null;
        }
    };

    if (loading) {
        return (
            <div className="flex items-center justify-center h-full">
                <div className="text-text-muted">Loading...</div>
            </div>
        );
    }

    return (
        <div className="flex flex-col h-full">
            {/* Header */}
            <div className="p-6 border-b border-border-subtle flex items-center justify-between">
                <div>
                    <h1 className="text-2xl font-semibold text-text-default">
                        {t('dashboard.title', 'Team Collaboration')}
                    </h1>
                    <p className="text-text-muted mt-1">
                        {t('dashboard.subtitle', 'Unified multi-source team management')}
                    </p>
                </div>
                <div className="flex gap-2">
                    <button
                        onClick={handleRefreshAll}
                        disabled={refreshing}
                        className="p-2 rounded-lg hover:bg-background-muted text-text-muted hover:text-text-default"
                        title={t('dashboard.refreshAll', 'Refresh all sources')}
                    >
                        <RefreshCw size={18} className={refreshing ? 'animate-spin' : ''} />
                    </button>
                    <button
                        onClick={() => setShowAddDialog(true)}
                        className="flex items-center gap-1 px-3 py-2 bg-teal-600 text-white rounded-lg hover:bg-teal-700"
                    >
                        <Plus size={16} />
                        {t('dashboard.addSource', 'Add Source')}
                    </button>
                </div>
            </div>

            {/* Content */}
            <div className="flex-1 p-6 overflow-y-auto">
                {/* Source summary cards */}
                <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-8">
                    {/* Local card */}
                    {localSources.length > 0 && (
                        <div
                            className="p-5 rounded-xl border-2 border-border-subtle hover:border-gray-400 hover:shadow-lg transition-all cursor-pointer"
                            onClick={() => localSources[0] && onSelectSource?.(localSources[0])}
                        >
                            <div className="flex items-start gap-4">
                                <div className="p-3 rounded-xl bg-gray-100 dark:bg-gray-800">
                                    <Monitor size={24} className="text-gray-600 dark:text-gray-400" />
                                </div>
                                <div className="flex-1">
                                    <h3 className="font-semibold text-text-default">
                                        {t('dashboard.local', 'Local')}
                                    </h3>
                                    <p className="text-sm mt-1">
                                        <span className={localSources[0]?.status === 'online' ? 'text-green-600' : 'text-gray-500'}>
                                            ● {localSources[0]?.status || 'offline'}
                                        </span>
                                    </p>
                                </div>
                            </div>
                        </div>
                    )}

                    {/* Cloud card */}
                    <button
                        onClick={onNavigateCloud}
                        className="p-5 rounded-xl border-2 border-border-subtle hover:border-blue-500 hover:shadow-lg transition-all text-left"
                    >
                        <div className="flex items-start gap-4">
                            <div className="p-3 rounded-xl bg-blue-100 dark:bg-blue-900/30">
                                <Cloud size={24} className="text-blue-600 dark:text-blue-400" />
                            </div>
                            <div className="flex-1">
                                <h3 className="font-semibold text-text-default flex items-center gap-2">
                                    {t('dashboard.cloudServers', 'Cloud Servers')}
                                    <ArrowRight size={16} className="text-text-muted opacity-50" />
                                </h3>
                                <div className="mt-2 space-y-1">
                                    <p className="text-sm text-text-muted">
                                        {t('dashboard.connectedCount', 'Connected: {{count}}', {
                                            count: cloudSources.length,
                                        })}
                                    </p>
                                    {cloudSources.length > 0 && (
                                        <p className="text-sm">
                                            <span className="text-green-600 dark:text-green-400">
                                                ● {cloudOnlineCount} {t('dashboard.online', 'online')}
                                            </span>
                                        </p>
                                    )}
                                </div>
                            </div>
                        </div>
                    </button>

                    {/* LAN card */}
                    <button
                        onClick={onNavigateLan}
                        className="p-5 rounded-xl border-2 border-border-subtle hover:border-green-500 hover:shadow-lg transition-all text-left"
                    >
                        <div className="flex items-start gap-4">
                            <div className="p-3 rounded-xl bg-green-100 dark:bg-green-900/30">
                                <Wifi size={24} className="text-green-600 dark:text-green-400" />
                            </div>
                            <div className="flex-1">
                                <h3 className="font-semibold text-text-default flex items-center gap-2">
                                    {t('dashboard.lanConnections', 'LAN Connections')}
                                    <ArrowRight size={16} className="text-text-muted opacity-50" />
                                </h3>
                                <div className="mt-2 space-y-1">
                                    <p className="text-sm text-text-muted">
                                        {t('dashboard.connectedCount', 'Connected: {{count}}', {
                                            count: lanSources.length,
                                        })}
                                    </p>
                                    {lanSources.length > 0 && (
                                        <p className="text-sm">
                                            <span className="text-green-600 dark:text-green-400">
                                                ● {lanOnlineCount} {t('dashboard.online', 'online')}
                                            </span>
                                        </p>
                                    )}
                                </div>
                            </div>
                        </div>
                    </button>
                </div>

                {/* Browse Resources */}
                {onNavigateResources && (
                    <div className="mb-8">
                        <h2 className="flex items-center gap-2 text-lg font-semibold text-text-default mb-4">
                            {t('dashboard.browseResources', 'Browse Resources')}
                        </h2>
                        <div className="grid grid-cols-3 gap-3">
                            <button
                                onClick={() => onNavigateResources('skill')}
                                className="p-4 rounded-lg border border-border-subtle hover:border-purple-500 hover:bg-purple-50 dark:hover:bg-purple-900/10 transition-all text-left"
                            >
                                <div className="flex items-center gap-3">
                                    <div className="p-2 rounded-lg bg-purple-100 dark:bg-purple-900/30">
                                        <Sparkles size={20} className="text-purple-600 dark:text-purple-400" />
                                    </div>
                                    <div>
                                        <p className="font-medium">{t('dashboard.skills', 'Skills')}</p>
                                        <p className="text-xs text-text-muted">{t('dashboard.skillsDesc', 'Browse all skills')}</p>
                                    </div>
                                </div>
                            </button>
                            <button
                                onClick={() => onNavigateResources('recipe')}
                                className="p-4 rounded-lg border border-border-subtle hover:border-amber-500 hover:bg-amber-50 dark:hover:bg-amber-900/10 transition-all text-left"
                            >
                                <div className="flex items-center gap-3">
                                    <div className="p-2 rounded-lg bg-amber-100 dark:bg-amber-900/30">
                                        <Book size={20} className="text-amber-600 dark:text-amber-400" />
                                    </div>
                                    <div>
                                        <p className="font-medium">{t('dashboard.recipes', 'Recipes')}</p>
                                        <p className="text-xs text-text-muted">{t('dashboard.recipesDesc', 'Browse all recipes')}</p>
                                    </div>
                                </div>
                            </button>
                            <button
                                onClick={() => onNavigateResources('extension')}
                                className="p-4 rounded-lg border border-border-subtle hover:border-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/10 transition-all text-left"
                            >
                                <div className="flex items-center gap-3">
                                    <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/30">
                                        <Puzzle size={20} className="text-blue-600 dark:text-blue-400" />
                                    </div>
                                    <div>
                                        <p className="font-medium">{t('dashboard.extensions', 'Extensions')}</p>
                                        <p className="text-xs text-text-muted">{t('dashboard.extensionsDesc', 'Browse all extensions')}</p>
                                    </div>
                                </div>
                            </button>
                        </div>
                    </div>
                )}

                {/* Recent teams */}
                <div>
                    <h2 className="flex items-center gap-2 text-lg font-semibold text-text-default mb-4">
                        <Star size={18} className="text-yellow-500" />
                        {t('dashboard.recentTeams', 'Recent Teams')}
                    </h2>

                    {recentTeams.length === 0 ? (
                        <div className="text-center py-8 text-text-muted">
                            <Clock size={32} className="mx-auto mb-2 opacity-50" />
                            <p>{t('dashboard.noRecentTeams', 'No recently accessed teams')}</p>
                            <p className="text-sm mt-1">
                                {t('dashboard.noRecentTeamsHint', 'Teams you visit will appear here for quick access')}
                            </p>
                        </div>
                    ) : (
                        <div className="space-y-2">
                            {recentTeams.map((team) => (
                                <button
                                    key={`${team.sourceType}-${team.sourceId}-${team.teamId}`}
                                    onClick={() => onSelectRecentTeam(team)}
                                    className="w-full p-3 rounded-lg border border-border-subtle hover:border-teal-500 hover:bg-teal-50 dark:hover:bg-teal-900/10 transition-all text-left"
                                >
                                    <div className="flex items-center justify-between">
                                        <div className="flex items-center gap-3">
                                            <div
                                                className={`p-1.5 rounded ${
                                                    team.sourceType === 'cloud'
                                                        ? 'bg-blue-100 dark:bg-blue-900/30'
                                                        : team.sourceType === 'lan'
                                                            ? 'bg-green-100 dark:bg-green-900/30'
                                                            : 'bg-gray-100 dark:bg-gray-800'
                                                }`}
                                            >
                                                {getSourceIcon(team.sourceType)}
                                            </div>
                                            <div>
                                                <p className="font-medium text-text-default">{team.teamName}</p>
                                                <p className="text-xs text-text-muted">{team.sourceName}</p>
                                            </div>
                                        </div>
                                        <div className="text-xs text-text-muted">
                                            {formatRelativeTime(team.lastAccessed, t)}
                                        </div>
                                    </div>
                                </button>
                            ))}
                        </div>
                    )}
                </div>
            </div>

            {/* Add Source Dialog */}
            <AddSourceDialog
                isOpen={showAddDialog}
                onClose={() => setShowAddDialog(false)}
                onSuccess={handleAddSuccess}
            />
        </div>
    );
};

export default UnifiedDashboard;
