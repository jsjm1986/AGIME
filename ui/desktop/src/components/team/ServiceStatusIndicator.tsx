import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Wifi, WifiOff, RefreshCw, Cloud, Monitor } from 'lucide-react';
import { checkServiceHealth, ServiceHealth, getTeamConnectionMode, TeamConnectionMode } from './api';

interface ServiceStatusIndicatorProps {
  className?: string;
  showLabel?: boolean;
  autoRefresh?: boolean;
  refreshInterval?: number; // in milliseconds
}

const ServiceStatusIndicator: React.FC<ServiceStatusIndicatorProps> = ({
  className = '',
  showLabel = true,
  autoRefresh = true,
  refreshInterval = 30000, // 30 seconds
}) => {
  const { t } = useTranslation('team');
  const [health, setHealth] = useState<ServiceHealth | null>(null);
  const [isChecking, setIsChecking] = useState(false);
  const [connectionMode, setConnectionMode] = useState<TeamConnectionMode>(null);

  const checkHealth = useCallback(async () => {
    setIsChecking(true);
    try {
      const mode = getTeamConnectionMode();
      setConnectionMode(mode);
      const result = await checkServiceHealth();
      setHealth(result);
    } catch {
      setHealth({ online: false, error: 'Check failed' });
    } finally {
      setIsChecking(false);
    }
  }, []);

  useEffect(() => {
    // Initial check
    checkHealth();

    // Auto refresh
    if (autoRefresh) {
      const interval = setInterval(checkHealth, refreshInterval);
      return () => clearInterval(interval);
    }
    return undefined;
  }, [checkHealth, autoRefresh, refreshInterval]);

  const getStatusColor = () => {
    if (isChecking) return 'text-yellow-500';
    if (!health) return 'text-gray-400';
    return health.online ? 'text-green-500' : 'text-red-500';
  };

  const getStatusBgColor = () => {
    if (isChecking) return 'bg-yellow-500/10';
    if (!health) return 'bg-gray-500/10';
    return health.online ? 'bg-green-500/10' : 'bg-red-500/10';
  };

  const getStatusText = () => {
    if (isChecking) return t('service.checking', 'Checking...');
    if (!health) return t('service.unknown', 'Unknown');

    // Get mode label
    const getModeLabel = () => {
      if (connectionMode === 'lan') return t('service.modeLan', 'LAN');
      if (connectionMode === 'cloud') return t('service.modeCloud', 'Cloud');
      return t('service.modeLocal', 'Local');
    };

    if (health.online) {
      const modeLabel = getModeLabel();
      return health.latency
        ? `${modeLabel} (${health.latency}ms)`
        : `${modeLabel} - ${t('service.online', 'Online')}`;
    }
    return t('service.offline', 'Offline');
  };

  // Get icon based on connection mode
  const getModeIcon = () => {
    if (connectionMode === 'lan') return Wifi;
    if (connectionMode === 'cloud') return Cloud;
    return Monitor;
  };

  const StatusIcon = health?.online ? getModeIcon() : WifiOff;

  return (
    <div
      className={`flex items-center gap-2 px-3 py-1.5 rounded-lg ${getStatusBgColor()} ${className}`}
      title={health?.error || getStatusText()}
    >
      {isChecking ? (
        <RefreshCw size={14} className={`${getStatusColor()} animate-spin`} />
      ) : (
        <StatusIcon size={14} className={getStatusColor()} />
      )}
      {showLabel && (
        <span className={`text-xs font-medium ${getStatusColor()}`}>
          {getStatusText()}
        </span>
      )}
      {!isChecking && (
        <button
          onClick={checkHealth}
          className="ml-1 p-0.5 rounded hover:bg-white/10 transition-colors"
          title={t('service.refreshStatus', 'Refresh status')}
        >
          <RefreshCw size={12} className="text-gray-400 hover:text-gray-300" />
        </button>
      )}
    </div>
  );
};

export default ServiceStatusIndicator;
