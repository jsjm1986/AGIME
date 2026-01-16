import React from 'react';
import { useTranslation } from 'react-i18next';
import { Monitor, Wifi, WifiOff, MoreVertical, Trash2, RefreshCw, ArrowRight } from 'lucide-react';
import { LANConnection } from '../types';
import {
    DropdownMenu,
    DropdownMenuContent,
    DropdownMenuItem,
    DropdownMenuTrigger,
} from '../../ui/dropdown-menu';

interface LANDeviceCardProps {
    connection: LANConnection;
    onSelect: () => void;
    onRemove: () => void;
    onRefresh: () => void;
}

const LANDeviceCard: React.FC<LANDeviceCardProps> = ({
    connection,
    onSelect,
    onRemove,
    onRefresh,
}) => {
    const { t } = useTranslation('team');

    const getStatusIcon = () => {
        switch (connection.status) {
            case 'connected':
                return <Wifi size={14} className="text-green-500" />;
            case 'connecting':
                return <RefreshCw size={14} className="text-yellow-500 animate-spin" />;
            case 'disconnected':
            case 'error':
                return <WifiOff size={14} className="text-red-500" />;
            default:
                return <WifiOff size={14} className="text-gray-400" />;
        }
    };

    const getStatusText = () => {
        switch (connection.status) {
            case 'connected':
                return t('lan.online', 'Online');
            case 'connecting':
                return t('lan.connecting', 'Connecting...');
            case 'disconnected':
                return t('lan.offline', 'Offline');
            case 'error':
                return connection.lastError || t('lan.error', 'Error');
            default:
                return t('lan.unknown', 'Unknown');
        }
    };

    const isOnline = connection.status === 'connected';

    return (
        <div
            onClick={() => isOnline && onSelect()}
            className={`
        p-4 rounded-lg border transition-all
        ${isOnline
                    ? 'border-border-subtle hover:border-green-500 hover:bg-green-50 dark:hover:bg-green-900/10 cursor-pointer'
                    : 'border-border-subtle opacity-70 cursor-not-allowed'
                }
      `}
        >
            <div className="flex items-start justify-between">
                <div className="flex items-start gap-3">
                    <div className={`
            p-2 rounded-lg
            ${isOnline
                            ? 'bg-green-100 dark:bg-green-900/30'
                            : 'bg-gray-100 dark:bg-gray-800'
                        }
          `}>
                        <Monitor size={20} className={
                            isOnline
                                ? 'text-green-600 dark:text-green-400'
                                : 'text-gray-500'
                        } />
                    </div>
                    <div className="flex-1 min-w-0">
                        <h3 className="font-medium text-text-default truncate flex items-center gap-2">
                            {connection.name}
                            {isOnline && (
                                <ArrowRight size={14} className="text-text-muted opacity-50" />
                            )}
                        </h3>
                        <p className="text-xs text-text-muted mt-0.5 truncate">
                            {connection.host}:{connection.port}
                        </p>
                        <div className="flex items-center gap-2 mt-2">
                            {getStatusIcon()}
                            <span className={`text-xs ${isOnline ? 'text-green-600 dark:text-green-400' :
                                    connection.status === 'error' ? 'text-red-600 dark:text-red-400' :
                                        'text-text-muted'
                                }`}>
                                {getStatusText()}
                            </span>
                            {isOnline && connection.teamsCount !== undefined && connection.teamsCount > 0 && (
                                <>
                                    <span className="text-text-muted">•</span>
                                    <span className="text-xs text-text-muted">
                                        {t('lan.teamsCount', '{{count}} teams', { count: connection.teamsCount })}
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
                            {t('lan.refresh', 'Refresh')}
                        </DropdownMenuItem>
                        <DropdownMenuItem
                            onClick={(e) => {
                                e.stopPropagation();
                                onRemove();
                            }}
                            className="text-red-600 dark:text-red-400"
                        >
                            <Trash2 size={14} className="mr-2" />
                            {t('lan.remove', 'Remove')}
                        </DropdownMenuItem>
                    </DropdownMenuContent>
                </DropdownMenu>
            </div>
        </div>
    );
};

export default LANDeviceCard;
