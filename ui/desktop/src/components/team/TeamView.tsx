import React, { useState, useCallback, useEffect } from 'react';
import TeamModeSelector, { TeamMode } from './TeamModeSelector';
import CloudTeamView from './CloudTeamView';
import { LANTeamView } from './lan';
import UnifiedDashboard from './UnifiedDashboard';
import { RecentTeam } from './recentTeamsStore';
import { getTeamConnectionMode } from './api';
import { getServers, migrateFromOldStorage } from './servers';
import { getConnections } from './lan';

interface TeamViewProps {
  onClose?: () => void;
}

// Storage key for selected mode
const MODE_STORAGE_KEY = 'AGIME_TEAM_VIEW_MODE';

const TeamView: React.FC<TeamViewProps> = () => {
  const [selectedMode, setSelectedMode] = useState<TeamMode | null>(null);
  const [isInitialized, setIsInitialized] = useState(false);

  // Initialize: check for existing connections and decide initial view
  useEffect(() => {
    // Migrate old single-server storage to new format
    migrateFromOldStorage();

    // Check if user has existing connections
    const servers = getServers();
    const lanConnections = getConnections();
    const connectionMode = getTeamConnectionMode();
    const savedMode = localStorage.getItem(MODE_STORAGE_KEY) as TeamMode | null;

    // If user has connections, show dashboard by default
    if (servers.length > 0 || lanConnections.length > 0) {
      setSelectedMode(savedMode || 'dashboard');
    } else if (connectionMode === 'cloud') {
      setSelectedMode('cloud');
    } else if (connectionMode === 'lan') {
      setSelectedMode('lan');
    }
    // Otherwise, show mode selector

    setIsInitialized(true);
  }, []);

  const handleSelectMode = useCallback((mode: TeamMode) => {
    setSelectedMode(mode);
    try {
      localStorage.setItem(MODE_STORAGE_KEY, mode);
    } catch {
      // Ignore storage errors
    }
  }, []);

  const handleBackToModeSelector = useCallback(() => {
    setSelectedMode(null);
    try {
      localStorage.removeItem(MODE_STORAGE_KEY);
    } catch {
      // Ignore storage errors
    }
  }, []);

  const handleSelectRecentTeam = useCallback((team: RecentTeam) => {
    // Navigate to the appropriate view based on source type
    if (team.sourceType === 'cloud') {
      // Set the active server and navigate to cloud view
      try {
        localStorage.setItem('AGIME_TEAM_ACTIVE_SERVER', team.sourceId);
      } catch {
        // Ignore
      }
      setSelectedMode('cloud');
    } else if (team.sourceType === 'lan') {
      // Set the active LAN connection and navigate to LAN view
      try {
        localStorage.setItem('AGIME_TEAM_ACTIVE_LAN_CONNECTION', team.sourceId);
      } catch {
        // Ignore
      }
      setSelectedMode('lan');
    }
  }, []);

  // Show loading while initializing
  if (!isInitialized) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>
      </div>
    );
  }

  // Show mode selector if no mode selected
  if (!selectedMode) {
    return <TeamModeSelector onSelectMode={handleSelectMode} />;
  }

  // Dashboard mode
  if (selectedMode === 'dashboard') {
    return (
      <UnifiedDashboard
        onNavigateCloud={() => handleSelectMode('cloud')}
        onNavigateLan={() => handleSelectMode('lan')}
        onSelectRecentTeam={handleSelectRecentTeam}
      />
    );
  }

  // Cloud mode
  if (selectedMode === 'cloud') {
    return <CloudTeamView onBack={handleBackToModeSelector} />;
  }

  // LAN mode
  if (selectedMode === 'lan') {
    return <LANTeamView onBack={handleBackToModeSelector} />;
  }

  // Fallback
  return <TeamModeSelector onSelectMode={handleSelectMode} />;
};

export default TeamView;
