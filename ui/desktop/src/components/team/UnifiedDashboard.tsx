import React from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, Wifi, ArrowRight, Star, Clock } from 'lucide-react';
import { RecentTeam, getRecentTeams, formatRelativeTime } from './recentTeamsStore';
import { getServers } from './servers';
import { getConnections } from './lan';

interface UnifiedDashboardProps {
    onNavigateCloud: () => void;
    onNavigateLan: () => void;
    onSelectRecentTeam: (team: RecentTeam) => void;
}

const UnifiedDashboard: React.FC<UnifiedDashboardProps> = ({
    onNavigateCloud,
    onNavigateLan,
    onSelectRecentTeam,
}) => {
    const { t } = useTranslation('team');

    // Get cloud servers and LAN connections
    const cloudServers = getServers();
    const lanConnections = getConnections();
    const recentTeams = getRecentTeams().slice(0, 5); // Show top 5

    const cloudOnlineCount = cloudServers.filter((s) => s.status === 'online').length;
    const lanOnlineCount = lanConnections.filter((c) => c.status === 'connected').length;
    const lanOfflineCount = lanConnections.filter((c) => c.status !== 'connected').length;

    return (
        <div className="flex flex-col h-full">
            {/* Header */}
            <div className="p-6 border-b border-border-subtle">
                <h1 className="text-2xl font-semibold text-text-default">
                    {t('dashboard.title', 'Team Collaboration')}
                </h1>
                <p className="text-text-muted mt-1">
                    {t('dashboard.subtitle', 'Manage your cloud servers and LAN connections')}
                </p>
            </div>

            {/* Content */}
            <div className="flex-1 p-6 overflow-y-auto">
                {/* Connection summary cards */}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-8">
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
                                            count: cloudServers.length,
                                        })}
                                    </p>
                                    {cloudServers.length > 0 && (
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
                                            count: lanConnections.length,
                                        })}
                                    </p>
                                    {lanConnections.length > 0 && (
                                        <p className="text-sm">
                                            <span className="text-green-600 dark:text-green-400">
                                                ● {lanOnlineCount} {t('dashboard.online', 'online')}
                                            </span>
                                            {lanOfflineCount > 0 && (
                                                <span className="text-text-muted ml-2">
                                                    ○ {lanOfflineCount} {t('dashboard.offline', 'offline')}
                                                </span>
                                            )}
                                        </p>
                                    )}
                                </div>
                            </div>
                        </div>
                    </button>
                </div>

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
                                                className={`p-1.5 rounded ${team.sourceType === 'cloud'
                                                    ? 'bg-blue-100 dark:bg-blue-900/30'
                                                    : 'bg-green-100 dark:bg-green-900/30'
                                                    }`}
                                            >
                                                {team.sourceType === 'cloud' ? (
                                                    <Cloud size={14} className="text-blue-500" />
                                                ) : (
                                                    <Wifi size={14} className="text-green-500" />
                                                )}
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
        </div>
    );
};

export default UnifiedDashboard;
