import React, { useState, useCallback, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowLeft } from 'lucide-react';
import { LANTeamView } from './lan';
import UnifiedDashboard from './UnifiedDashboard';
import { UnifiedResourceView } from './UnifiedResourceView';
import { RecentTeam } from './recentTeamsStore';
import { sourceManager } from './sources/sourceManager';
import type { DataSource, SourcedResource } from './sources/types';
import type { SharedSkill, SharedRecipe, SharedExtension, Team, TeamSummary } from './types';
import { installSkill, installRecipe, installExtension, getTeam } from './api';
import { toastService } from '../../toasts';
import ResourceDetailDialog from './ResourceDetailDialog';
import CloudSourceView from './CloudSourceView';
import TeamList from './TeamList';
import TeamDetail from './TeamDetail';

// View modes for the team interface
type ViewMode = 'dashboard' | 'cloud' | 'lan' | 'source-detail' | 'resources' | 'teams' | 'team-detail';

// Resource type for browsing
type ResourceType = 'skill' | 'recipe' | 'extension';
type AnyResource = SharedSkill | SharedRecipe | SharedExtension;

interface TeamViewProps {
  onClose?: () => void;
}

const TeamView: React.FC<TeamViewProps> = () => {
  const { t } = useTranslation('team');
  const [viewMode, setViewMode] = useState<ViewMode>('dashboard');
  const [isInitialized, setIsInitialized] = useState(false);
  const [resourceType, setResourceType] = useState<ResourceType>('skill');
  const [activeSource, setActiveSource] = useState<DataSource | null>(null);

  // Resource detail dialog state
  const [detailDialogOpen, setDetailDialogOpen] = useState(false);
  const [selectedResource, setSelectedResource] = useState<AnyResource | null>(null);

  // Team state
  const [selectedTeam, setSelectedTeam] = useState<Team | null>(null);
  const [teamSummary, setTeamSummary] = useState<TeamSummary | null>(null);
  const [isLoadingTeam, setIsLoadingTeam] = useState(false);
  const [teamError, setTeamError] = useState<string | null>(null);

  // Initialize: load sources
  useEffect(() => {
    const init = async () => {
      await sourceManager.initialize();
      setIsInitialized(true);
    };
    init();
  }, []);

  const handleSelectSource = useCallback((source: DataSource) => {
    setActiveSource(source);
    if (source.type === 'cloud') {
      setViewMode('cloud');
    } else if (source.type === 'lan') {
      setViewMode('lan');
    } else {
      setViewMode('source-detail');
    }
  }, []);

  const handleBackToDashboard = useCallback(() => {
    setViewMode('dashboard');
  }, []);

  const handleNavigateResources = useCallback((type: ResourceType) => {
    setResourceType(type);
    setViewMode('resources');
  }, []);

  const handleNavigateTeams = useCallback(() => {
    setViewMode('teams');
  }, []);

  const handleSelectTeam = useCallback(async (team: Team) => {
    setSelectedTeam(team);
    setViewMode('team-detail');
    setIsLoadingTeam(true);
    setTeamError(null);
    setTeamSummary(null);

    try {
      const summary = await getTeam(team.id);
      setTeamSummary(summary);
    } catch (err) {
      console.error('Failed to load team details:', err);
      setTeamError(t('teamDetailError', 'Failed to load team details'));
    } finally {
      setIsLoadingTeam(false);
    }
  }, [t]);

  const handleBackToTeams = useCallback(() => {
    setViewMode('teams');
  }, []);

  const handleSelectRecentTeam = useCallback((team: RecentTeam) => {
    // Find the source and navigate
    const source = sourceManager.getSource(team.sourceId);
    if (source) {
      handleSelectSource(source);
    } else if (team.sourceType === 'cloud') {
      setViewMode('cloud');
    } else if (team.sourceType === 'lan') {
      setViewMode('lan');
    }
  }, [handleSelectSource]);

  // Handle resource installation
  const handleInstallResource = useCallback(async (sourcedResource: SourcedResource<AnyResource>) => {
    const { resource } = sourcedResource;
    const loadingToastId = toastService.loading({
      title: t('resources.installing', 'Installing...'),
      msg: resource.name,
    });

    try {
      if (resourceType === 'skill') {
        await installSkill(resource.id);
      } else if (resourceType === 'recipe') {
        await installRecipe(resource.id);
      } else if (resourceType === 'extension') {
        await installExtension(resource.id);
      }

      toastService.dismiss(loadingToastId);
      toastService.success({
        title: t('resources.installSuccess', 'Installed successfully'),
        msg: resource.name,
      });
    } catch (error) {
      toastService.dismiss(loadingToastId);
      toastService.error({
        title: t('resources.installFailed', 'Installation failed'),
        msg: error instanceof Error ? error.message : String(error),
      });
    }
  }, [resourceType, t]);

  // Handle view resource details
  const handleViewResourceDetails = useCallback((sourcedResource: SourcedResource<AnyResource>) => {
    setSelectedResource(sourcedResource.resource);
    setDetailDialogOpen(true);
  }, []);

  // Get resource type label
  const getResourceTypeLabel = (type: ResourceType) => {
    switch (type) {
      case 'skill':
        return t('resources.skills', 'Skills');
      case 'recipe':
        return t('resources.recipes', 'Recipes');
      case 'extension':
        return t('resources.extensions', 'Extensions');
    }
  };

  // Show loading while initializing
  if (!isInitialized) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>
      </div>
    );
  }

  // Render main content based on view mode
  const renderContent = () => {
    // Dashboard mode (default)
    if (viewMode === 'dashboard') {
      return (
        <UnifiedDashboard
          onNavigateCloud={() => setViewMode('cloud')}
          onNavigateLan={() => setViewMode('lan')}
          onSelectRecentTeam={handleSelectRecentTeam}
          onSelectSource={handleSelectSource}
          onNavigateResources={handleNavigateResources}
        />
      );
    }

    // LAN mode
    if (viewMode === 'lan') {
      return <LANTeamView onBack={handleBackToDashboard} />;
    }

    // Resources browsing mode
    if (viewMode === 'resources') {
      return (
        <div className="flex flex-col h-full">
          {/* Header */}
          <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
            <button
              onClick={handleBackToDashboard}
              className="p-2 rounded-lg hover:bg-background-muted text-text-muted hover:text-text-default"
            >
              <ArrowLeft size={20} />
            </button>
            <h2 className="font-semibold text-lg text-text-default">
              {getResourceTypeLabel(resourceType)}
            </h2>
            {/* Resource type tabs */}
            <div className="flex gap-1 ml-4">
              {(['skill', 'recipe', 'extension'] as ResourceType[]).map((type) => (
                <button
                  key={type}
                  onClick={() => setResourceType(type)}
                  className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${
                    resourceType === type
                      ? 'bg-teal-100 text-teal-700 dark:bg-teal-900/30 dark:text-teal-300'
                      : 'hover:bg-background-muted text-text-muted'
                  }`}
                >
                  {getResourceTypeLabel(type)}
                </button>
              ))}
            </div>
          </div>
          {/* Resource view */}
          <div className="flex-1 overflow-y-auto p-4">
            <UnifiedResourceView
              resourceType={resourceType}
              onInstall={handleInstallResource}
              onViewDetails={handleViewResourceDetails}
            />
          </div>
        </div>
      );
    }

    // Cloud mode
    if (viewMode === 'cloud') {
      return (
        <CloudSourceView
          source={activeSource}
          onBack={handleBackToDashboard}
          onNavigateResources={handleNavigateResources}
          onNavigateTeams={handleNavigateTeams}
        />
      );
    }

    // Teams list mode
    if (viewMode === 'teams') {
      return (
        <div className="flex flex-col h-full">
          <div className="p-4 border-b border-border-subtle">
            <div className="flex items-center gap-3">
              <button
                onClick={() => setViewMode('cloud')}
                className="p-2 rounded-lg hover:bg-background-muted"
              >
                <ArrowLeft size={20} />
              </button>
              <h1 className="text-xl font-semibold">{t('team.teams', 'Teams')}</h1>
            </div>
          </div>
          <div className="flex-1 overflow-auto p-4">
            <TeamList
              onSelectTeam={handleSelectTeam}
              selectedTeamId={selectedTeam?.id || null}
            />
          </div>
        </div>
      );
    }

    // Team detail mode
    if (viewMode === 'team-detail' && selectedTeam) {
      // Show loading or error state, or TeamDetail when data is ready
      if (isLoadingTeam || !teamSummary) {
        return (
          <div className="flex items-center justify-center h-full">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-teal-500"></div>
          </div>
        );
      }

      return (
        <TeamDetail
          teamSummary={teamSummary}
          isLoading={isLoadingTeam}
          error={teamError}
          onBack={handleBackToTeams}
          onRetry={() => handleSelectTeam(selectedTeam)}
          onTeamDeleted={() => {
            setViewMode('teams');
            setSelectedTeam(null);
            setTeamSummary(null);
          }}
          onTeamUpdated={(updated) => setTeamSummary(updated)}
        />
      );
    }

    // Fallback to dashboard
    return (
      <UnifiedDashboard
        onNavigateCloud={() => setViewMode('cloud')}
        onNavigateLan={() => setViewMode('lan')}
        onSelectRecentTeam={handleSelectRecentTeam}
        onSelectSource={handleSelectSource}
        onNavigateResources={handleNavigateResources}
      />
    );
  };

  return (
    <>
      {renderContent()}

      {/* Resource Detail Dialog */}
      <ResourceDetailDialog
        open={detailDialogOpen}
        onOpenChange={setDetailDialogOpen}
        resourceType={resourceType}
        resource={selectedResource}
      />
    </>
  );
};

export default TeamView;
