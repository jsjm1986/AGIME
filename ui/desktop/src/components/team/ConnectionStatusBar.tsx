import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, Wifi, RefreshCw, CheckCircle, AlertCircle } from 'lucide-react';
import { getServers } from './servers';
import { getConnections, checkAllConnections as checkLanConnections } from './lan';

interface ConnectionStatusBarProps {
    onRefresh?: () => void;
}

const ConnectionStatusBar: React.FC<ConnectionStatusBarProps> = ({ onRefresh }) => {
    const { t } = useTranslation('team');
    const [isRefreshing, setIsRefreshing] = useState(false);
    const [cloudServers, setCloudServers] = useState<ReturnType<typeof getServers>>([]);
    const [lanConnections, setLanConnections] = useState<ReturnType<typeof getConnections>>([]);

    const loadData = useCallback(() => {
        setCloudServers(getServers());
        setLanConnections(getConnections());
    }, []);

    useEffect(() => {
        loadData();
        // Refresh every 30 seconds
        const interval = setInterval(loadData, 30000);
        return () => clearInterval(interval);
    }, [loadData]);

    const cloudOnline = cloudServers.filter((s) => s.status === 'online').length;
    const cloudTotal = cloudServers.length;
    const lanOnline = lanConnections.filter((c) => c.status === 'connected').length;
    const lanTotal = lanConnections.length;

    const allOnline = cloudOnline === cloudTotal && lanOnline === lanTotal && (cloudTotal + lanTotal > 0);
    const hasIssues = (cloudTotal > cloudOnline) || (lanTotal > lanOnline);

    const handleRefresh = async () => {
        setIsRefreshing(true);
        try {
            // Check all LAN connections
            await checkLanConnections();
            loadData();
            onRefresh?.();
        } finally {
            setIsRefreshing(false);
        }
    };

    if (cloudTotal === 0 && lanTotal === 0) {
        return null; // Don't show if no connections
    }

    return (
        <div className="flex items-center gap-4 px-4 py-2 bg-background-muted border-b border-border-subtle">
            {/* Cloud status */}
            {cloudTotal > 0 && (
                <div className="flex items-center gap-2 text-sm">
                    <Cloud size={14} className="text-blue-500" />
                    <span className="text-text-muted">
                        {t('statusBar.cloud', 'Cloud')}:
                    </span>
                    <span className={cloudOnline === cloudTotal ? 'text-green-600' : 'text-yellow-600'}>
                        {cloudOnline}/{cloudTotal}
                    </span>
                </div>
            )}

            {/* LAN status */}
            {lanTotal > 0 && (
                <div className="flex items-center gap-2 text-sm">
                    <Wifi size={14} className="text-green-500" />
                    <span className="text-text-muted">
                        {t('statusBar.lan', 'LAN')}:
                    </span>
                    <span className={lanOnline === lanTotal ? 'text-green-600' : 'text-yellow-600'}>
                        {lanOnline}/{lanTotal}
                    </span>
                </div>
            )}

            {/* Status icon */}
            <div className="flex items-center gap-1 ml-auto">
                {allOnline && (
                    <CheckCircle size={14} className="text-green-500" />
                )}
                {hasIssues && (
                    <AlertCircle size={14} className="text-yellow-500" />
                )}

                {/* Refresh button */}
                <button
                    onClick={handleRefresh}
                    disabled={isRefreshing}
                    className="p-1 rounded hover:bg-background-default text-text-muted hover:text-text-default transition-colors"
                    title={t('statusBar.refresh', 'Refresh connections')}
                >
                    <RefreshCw
                        size={14}
                        className={isRefreshing ? 'animate-spin' : ''}
                    />
                </button>
            </div>
        </div>
    );
};

export default ConnectionStatusBar;
