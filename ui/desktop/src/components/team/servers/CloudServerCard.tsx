import React from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, Wifi, WifiOff, MoreVertical, Trash2, Settings, RefreshCw } from 'lucide-react';
import { CloudServer } from '../types';
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuTrigger,
} from '../../ui/dropdown-menu';

interface CloudServerCardProps {
    server: CloudServer;
    isActive: boolean;
    onSelect: () => void;
    onRemove: () => void;
    onRefresh: () => void;
    onSettings?: () => void;
}

const CloudServerCard: React.FC<CloudServerCardProps> = ({
    server,
    isActive,
    onSelect,
    onRemove,
    onRefresh,
    onSettings,
}) => {
    const { t } = useTranslation('team');

    const getStatusIcon = () => {
        switch (server.status) {
            case 'online':
                return <Wifi size={14} className="text-green-500" />;
            case 'connecting':
                return <RefreshCw size={14} className="text-yellow-500 animate-spin" />;
            case 'offline':
            case 'error':
                return <WifiOff size={14} className="text-red-500" />;
            default:
                return <WifiOff size={14} className="text-gray-400" />;
        }
    };

    const getStatusText = () => {
        switch (server.status) {
            case 'online':
                return t('server.online', 'Online');
            case 'connecting':
                return t('server.connecting', 'Connecting...');
            case 'offline':
                return t('server.offline', 'Offline');
            case 'error':
                return server.lastError || t('server.error', 'Error');
            default:
                return t('server.unknown', 'Unknown');
        }
    };

    const formatUrl = (url: string) => {
        try {
            const parsed = new URL(url);
            return parsed.host;
        } catch {
            return url;
        }
    };

    return (
        <div
            onClick={onSelect}
            className={`
        p-4 rounded-lg border cursor-pointer transition-all
        ${isActive
                    ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20'
                    : 'border-border-subtle hover:border-border-default hover:bg-background-muted'
                }
      `}
        >
            <div className="flex items-start justify-between">
                <div className="flex items-start gap-3">
                    <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/30">
                        <Cloud size={20} className="text-blue-600 dark:text-blue-400" />
                    </div>
                    <div className="flex-1 min-w-0">
                        <h3 className="font-medium text-text-default truncate">{server.name}</h3>
                        <p className="text-xs text-text-muted mt-0.5 truncate">
                            {formatUrl(server.url)}
                        </p>
                        {server.userEmail && (
                            <p className="text-xs text-text-muted mt-0.5 truncate">
                                {server.userEmail}
                            </p>
                        )}
                        <div className="flex items-center gap-2 mt-2">
                            {getStatusIcon()}
                            <span className={`text-xs ${server.status === 'online' ? 'text-green-600 dark:text-green-400' :
                                    server.status === 'error' ? 'text-red-600 dark:text-red-400' :
                                        'text-text-muted'
                                }`}>
                                {getStatusText()}
                            </span>
                            {server.status === 'online' && server.teamsCount > 0 && (
                                <>
                                    <span className="text-text-muted">•</span>
                                    <span className="text-xs text-text-muted">
                                        {t('server.teamsCount', '{{count}} teams', { count: server.teamsCount })}
                                    </span>
                                </>
                            )}
                        </div>
                    </div>
                </div>

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
                                onRefresh();
                            }}
                        >
                            <RefreshCw size={14} className="mr-2" />
                            {t('server.refresh', 'Refresh')}
                        </DropdownMenuItem>
                        {onSettings && (
                            <DropdownMenuItem
                                onClick={(e) => {
                                    e.stopPropagation();
                                    onSettings();
                                }}
                            >
                                <Settings size={14} className="mr-2" />
                                {t('server.settings', 'Settings')}
                            </DropdownMenuItem>
                        )}
                        <DropdownMenuItem
                            onClick={(e) => {
                                e.stopPropagation();
                                onRemove();
                            }}
                            className="text-red-600 dark:text-red-400"
                        >
                            <Trash2 size={14} className="mr-2" />
                            {t('server.remove', 'Remove')}
                        </DropdownMenuItem>
                    </DropdownMenuContent>
                </DropdownMenu>
            </div>
        </div>
    );
};

export default CloudServerCard;
