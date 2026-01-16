import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Wifi, MonitorOff, Home } from 'lucide-react';
import { LANConnection } from '../types';
import { Button } from '../../ui/button';
import LANDeviceCard from './LANDeviceCard';
import {
    getConnections,
    removeConnection,
    testLANConnection,
    updateConnection,
    updateConnectionStatus,
} from './lanStore';

interface LANDeviceListProps {
    onSelectConnection: (connection: LANConnection) => void;
    onAddConnection: () => void;
    onSelectLocalTeams: () => void;
}

const LANDeviceList: React.FC<LANDeviceListProps> = ({
    onSelectConnection,
    onAddConnection,
    onSelectLocalTeams,
}) => {
    const { t } = useTranslation('team');
    const [connections, setConnections] = useState<LANConnection[]>([]);
    const [isLoading, setIsLoading] = useState(true);

    // Load connections from localStorage
    const loadConnections = useCallback(() => {
        const saved = getConnections();
        setConnections(saved);
        setIsLoading(false);
    }, []);

    // Check connection status
    const checkConnectionStatus = useCallback(async (connection: LANConnection) => {
        updateConnectionStatus(connection.id, 'connecting');
        setConnections(getConnections());

        const result = await testLANConnection(connection.host, connection.port, connection.secretKey);

        if (result.success) {
            updateConnection(connection.id, {
                status: 'connected',
                teamsCount: result.teamsCount,
                lastOnline: new Date().toISOString(),
                lastError: undefined,
            });
        } else {
            updateConnectionStatus(connection.id, 'error', result.error);
        }

        setConnections(getConnections());
    }, []);

    // Initial load
    useEffect(() => {
        loadConnections();
    }, [loadConnections]);

    // Check all connections on mount
    useEffect(() => {
        if (!isLoading && connections.length > 0) {
            connections.forEach((conn) => {
                if (conn.status !== 'connecting') {
                    checkConnectionStatus(conn);
                }
            });
        }
    }, [isLoading]); // Only run once after initial load

    const handleRemoveConnection = (connectionId: string) => {
        const conn = connections.find((c) => c.id === connectionId);
        if (!conn) return;

        if (
            !confirm(
                t('lan.removeConfirm', 'Are you sure you want to remove "{{name}}"?', {
                    name: conn.name,
                })
            )
        ) {
            return;
        }

        removeConnection(connectionId);
        loadConnections();
    };

    const handleRefreshConnection = (connection: LANConnection) => {
        checkConnectionStatus(connection);
    };

    if (isLoading) {
        return (
            <div className="flex items-center justify-center h-64">
                <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-green-500"></div>
            </div>
        );
    }

    return (
        <div className="flex flex-col h-full">
            {/* Header */}
            <div className="flex items-center justify-between p-4 border-b border-border-subtle">
                <div className="flex items-center gap-3">
                    <Wifi size={24} className="text-green-500" />
                    <h1 className="text-xl font-semibold text-text-default">
                        {t('lan.title', 'LAN Connections')}
                    </h1>
                </div>
                <Button onClick={onAddConnection} className="flex items-center gap-2">
                    <Plus size={16} />
                    {t('lan.add', 'Add Connection')}
                </Button>
            </div>

            {/* Connection list */}
            <div className="flex-1 overflow-y-auto p-4">
                {/* Local Teams Card */}
                <div
                    onClick={onSelectLocalTeams}
                    className="mb-4 p-4 rounded-lg border-2 border-teal-500/20 bg-teal-500/5 hover:bg-teal-500/10 hover:border-teal-500/40 cursor-pointer transition-all"
                >
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-teal-500/20">
                            <Home size={24} className="text-teal-500" />
                        </div>
                        <div className="flex-1">
                            <h3 className="font-semibold text-text-default">
                                {t('lan.localTeams', 'My Local Teams')}
                            </h3>
                            <p className="text-sm text-text-muted">
                                {t('lan.localTeamsDescription', 'Manage teams on this device')}
                            </p>
                        </div>
                        <div className="text-text-muted">
                            →
                        </div>
                    </div>
                </div>

                {/* Divider */}
                {connections.length > 0 && (
                    <div className="flex items-center gap-3 my-4">
                        <div className="flex-1 border-t border-border-subtle"></div>
                        <span className="text-xs text-text-muted uppercase tracking-wider">
                            {t('lan.remoteConnections', 'LAN Connections')}
                        </span>
                        <div className="flex-1 border-t border-border-subtle"></div>
                    </div>
                )}

                {/* Connection list */}

                {connections.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-64 text-text-muted">
                        <MonitorOff size={48} className="mb-4 opacity-50" />
                        <p className="text-lg">{t('lan.noConnections', 'No LAN connections')}</p>
                        <p className="text-sm mt-2 text-center max-w-md">
                            {t(
                                'lan.noConnectionsDescription',
                                'Add a connection to access teams from another AGIME instance on your network'
                            )}
                        </p>
                        <Button onClick={onAddConnection} className="mt-4">
                            <Plus size={16} className="mr-2" />
                            {t('lan.addFirst', 'Add Your First Connection')}
                        </Button>
                    </div>
                ) : (
                    <div className="grid gap-4">
                        {connections.map((connection) => (
                            <LANDeviceCard
                                key={connection.id}
                                connection={connection}
                                onSelect={() => onSelectConnection(connection)}
                                onRemove={() => handleRemoveConnection(connection.id)}
                                onRefresh={() => handleRefreshConnection(connection)}
                            />
                        ))}
                    </div>
                )}
            </div>
        </div>
    );
};

export default LANDeviceList;
