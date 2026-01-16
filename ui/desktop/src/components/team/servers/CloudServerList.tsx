import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Cloud, ServerOff } from 'lucide-react';
import { CloudServer } from '../types';
import { Button } from '../../ui/button';
import CloudServerCard from './CloudServerCard';
import {
    getServers,
    getActiveServerId,
    setActiveServer,
    removeServer,
    updateServerStatus,
    testServerConnection,
    updateServer,
    migrateFromOldStorage,
} from './serverStore';

interface CloudServerListProps {
    onSelectServer: (server: CloudServer) => void;
    onAddServer: () => void;
}

const CloudServerList: React.FC<CloudServerListProps> = ({
    onSelectServer,
    onAddServer,
}) => {
    const { t } = useTranslation('team');
    const [servers, setServers] = useState<CloudServer[]>([]);
    const [activeServerId, setActiveServerId] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);

    // Load servers from localStorage
    const loadServers = useCallback(() => {
        const savedServers = getServers();
        setServers(savedServers);
        setActiveServerId(getActiveServerId());
        setIsLoading(false);
    }, []);

    // Check server status
    const checkServerStatus = useCallback(async (server: CloudServer) => {
        updateServerStatus(server.id, 'connecting');
        setServers(getServers());

        const result = await testServerConnection(server.url, server.apiKey);

        if (result.success) {
            updateServer(server.id, {
                status: 'online',
                userEmail: result.userEmail,
                displayName: result.displayName,
                userId: result.userId,
                lastError: undefined,
            });
        } else {
            updateServerStatus(server.id, 'error', result.error);
        }

        setServers(getServers());
    }, []);

    // Initial load and migration
    useEffect(() => {
        migrateFromOldStorage();
        loadServers();
    }, [loadServers]);

    // Check all servers on mount
    useEffect(() => {
        if (!isLoading && servers.length > 0) {
            servers.forEach((server) => {
                if (server.status !== 'connecting') {
                    checkServerStatus(server);
                }
            });
        }
    }, [isLoading]); // Only run once after initial load

    const handleSelectServer = (server: CloudServer) => {
        setActiveServer(server.id);
        setActiveServerId(server.id);
        onSelectServer(server);
    };

    const handleRemoveServer = (serverId: string) => {
        const server = servers.find((s) => s.id === serverId);
        if (!server) return;

        if (
            !confirm(
                t('server.removeConfirm', 'Are you sure you want to remove "{{name}}"?', {
                    name: server.name,
                })
            )
        ) {
            return;
        }

        removeServer(serverId);
        loadServers();
    };

    const handleRefreshServer = (server: CloudServer) => {
        checkServerStatus(server);
    };

    if (isLoading) {
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
                    <Cloud size={24} className="text-blue-500" />
                    <h1 className="text-xl font-semibold text-text-default">
                        {t('server.title', 'Cloud Servers')}
                    </h1>
                </div>
                <Button onClick={onAddServer} className="flex items-center gap-2">
                    <Plus size={16} />
                    {t('server.add', 'Add Server')}
                </Button>
            </div>

            {/* Server list */}
            <div className="flex-1 overflow-y-auto p-4">
                {servers.length === 0 ? (
                    <div className="flex flex-col items-center justify-center h-64 text-text-muted">
                        <ServerOff size={48} className="mb-4 opacity-50" />
                        <p className="text-lg">{t('server.noServers', 'No servers connected')}</p>
                        <p className="text-sm mt-2 text-center max-w-md">
                            {t(
                                'server.noServersDescription',
                                'Add a cloud server to collaborate with your team across locations'
                            )}
                        </p>
                        <Button onClick={onAddServer} className="mt-4">
                            <Plus size={16} className="mr-2" />
                            {t('server.addFirst', 'Add Your First Server')}
                        </Button>
                    </div>
                ) : (
                    <div className="grid gap-4">
                        {servers.map((server) => (
                            <CloudServerCard
                                key={server.id}
                                server={server}
                                isActive={activeServerId === server.id}
                                onSelect={() => handleSelectServer(server)}
                                onRemove={() => handleRemoveServer(server.id)}
                                onRefresh={() => handleRefreshServer(server)}
                            />
                        ))}
                    </div>
                )}
            </div>
        </div>
    );
};

export default CloudServerList;
