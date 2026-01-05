import React, { useEffect, useState, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  FolderOpen,
  Puzzle,
  Brain,
  MessageSquare,
  ArrowRight,
  Clock,
} from 'lucide-react';
import { useConfig } from '../ConfigContext';
import { listSessions, Session } from '../../api';
import { formatRelativeTime } from '../../utils/timeUtils';
import { cn } from '../../utils';
import { getConfigCompat } from '../../utils/envCompat';
import i18n from '../../i18n';

interface NewChatWelcomeProps {
  onSelectTip?: (prompt: string) => void;
}

// Context status bar component
const ContextStatusBar = React.memo(function ContextStatusBar({
  workingDir,
  extensionCount,
  hasMemory,
  onExtensionsClick,
}: {
  workingDir: string;
  extensionCount: number;
  hasMemory: boolean;
  onExtensionsClick: () => void;
}) {
  const { t } = useTranslation('welcome');

  // Get shortened working dir for display
  const shortWorkingDir = useMemo(() => {
    const parts = workingDir.split(/[/\\]/);
    if (parts.length <= 2) return workingDir;
    return '...' + parts.slice(-2).join('/');
  }, [workingDir]);

  return (
    <div className="flex flex-wrap items-center justify-center gap-4 sm:gap-6 px-6 py-3 bg-background-subtle/50 rounded-xl text-sm">
      {/* Working directory */}
      <div
        className="flex items-center gap-2 text-text-muted hover:text-text-default transition-colors cursor-default"
        title={workingDir}
      >
        <FolderOpen className="w-4 h-4 flex-shrink-0" />
        <span>{t('context.workingDir')}：</span>
        <span className="truncate max-w-[180px] sm:max-w-[250px]">{shortWorkingDir}</span>
      </div>

      <div className="w-px h-5 bg-border-subtle hidden sm:block" />

      {/* Extensions status */}
      <button
        onClick={onExtensionsClick}
        className="flex items-center gap-2 text-text-muted hover:text-text-default transition-colors"
      >
        <Puzzle className="w-4 h-4 flex-shrink-0" />
        <span>{t('context.extensions', { count: extensionCount })}</span>
      </button>

      <div className="w-px h-5 bg-border-subtle hidden sm:block" />

      {/* Memory status */}
      <div className="flex items-center gap-2 text-text-muted">
        <Brain className="w-4 h-4 flex-shrink-0" />
        <span>{hasMemory ? t('context.memory') : t('context.noMemory')}</span>
      </div>
    </div>
  );
});

// Session card component - larger and more prominent
const SessionCard = React.memo(function SessionCard({
  session,
  onClick,
}: {
  session: Session;
  onClick: () => void;
}) {
  // Get shortened working dir
  const shortPath = useMemo(() => {
    const parts = session.working_dir.split(/[/\\]/);
    if (parts.length <= 2) return session.working_dir;
    return '...' + parts.slice(-2).join('/');
  }, [session.working_dir]);

  const relativeTime = formatRelativeTime(
    Date.parse(session.updated_at) / 1000,
    i18n.language
  );

  return (
    <button
      onClick={onClick}
      className={cn(
        'p-4 rounded-xl border border-border-subtle',
        'hover:border-border-default hover:shadow-md hover:bg-background-subtle/30',
        'transition-all duration-200 text-left w-full',
        'bg-background-default group'
      )}
    >
      <div className="flex items-start gap-3">
        <div className="w-10 h-10 rounded-lg bg-background-subtle flex items-center justify-center flex-shrink-0 group-hover:bg-background-muted transition-colors">
          <MessageSquare className="w-5 h-5 text-text-muted" />
        </div>
        <div className="flex-1 min-w-0">
          <h4 className="font-medium text-text-default text-base truncate mb-1">
            {session.name}
          </h4>
          <p className="text-sm text-text-muted truncate mb-2" title={session.working_dir}>
            {shortPath}
          </p>
          <div className="flex items-center gap-1 text-xs text-text-muted">
            <Clock className="w-3 h-3" />
            <span>{relativeTime}</span>
          </div>
        </div>
      </div>
    </button>
  );
});

// Recent sessions section component - full width
const RecentSessionsSection = React.memo(function RecentSessionsSection({
  sessions,
  onSelectSession,
  onViewAll,
}: {
  sessions: Session[];
  onSelectSession: (sessionId: string) => void;
  onViewAll: () => void;
}) {
  const { t } = useTranslation('welcome');

  if (sessions.length === 0) {
    return null;
  }

  return (
    <div className="w-full max-w-6xl mx-auto px-4">
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <h3 className="text-base font-medium text-text-default">
          {t('recentSessions.title')}
        </h3>
        <button
          onClick={onViewAll}
          className="flex items-center gap-1.5 text-sm text-text-muted hover:text-text-default transition-colors group"
        >
          {t('recentSessions.viewAll')}
          <ArrowRight className="w-4 h-4 group-hover:translate-x-0.5 transition-transform" />
        </button>
      </div>

      {/* Session cards grid - responsive */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
        {sessions.map((session) => (
          <SessionCard
            key={session.id}
            session={session}
            onClick={() => onSelectSession(session.id)}
          />
        ))}
      </div>
    </div>
  );
});

// Empty state component - more prominent
const EmptyState = React.memo(function EmptyState() {
  const { t } = useTranslation('welcome');

  return (
    <div className="text-center py-12 px-4 max-w-md mx-auto">
      <div className="w-16 h-16 rounded-2xl bg-background-subtle flex items-center justify-center mx-auto mb-4">
        <MessageSquare className="w-8 h-8 text-text-muted" />
      </div>
      <p className="text-text-default text-lg font-medium mb-2">{t('recentSessions.empty')}</p>
      <p className="text-sm text-text-muted">{t('recentSessions.emptyDesc')}</p>
    </div>
  );
});

export const NewChatWelcome: React.FC<NewChatWelcomeProps> = React.memo(
  function NewChatWelcome({ onSelectTip: _onSelectTip }) {
    const { t } = useTranslation('welcome');
    const navigate = useNavigate();
    const { extensionsList, getExtensions } = useConfig();

    const [recentSessions, setRecentSessions] = useState<Session[]>([]);
    const [workingDir, setWorkingDir] = useState<string>('');
    const [isLoading, setIsLoading] = useState(true);

    // Calculate enabled extensions count
    const enabledExtensionsCount = useMemo(() => {
      return extensionsList.filter((ext) => ext.enabled).length;
    }, [extensionsList]);

    // Check if memory extension is enabled
    const hasMemory = useMemo(() => {
      return extensionsList.some(
        (ext) => ext.enabled && ext.name.toLowerCase() === 'memory'
      );
    }, [extensionsList]);

    // Load data on mount
    useEffect(() => {
      const loadData = async () => {
        setIsLoading(true);
        try {
          // Load recent sessions
          const response = await listSessions<true>({
            throwOnError: true,
          });
          // Limit to 4 sessions for display
          setRecentSessions(response.data.sessions.slice(0, 4));
        } catch (error) {
          console.error('Failed to load recent sessions:', error);
          // Keep empty sessions array - will show empty state
        }

        try {
          // Get working directory from env
          const dir = (getConfigCompat('WORKING_DIR') as string) || '.';
          setWorkingDir(dir);

          // Refresh extensions list
          await getExtensions(true);
        } catch (error) {
          console.error('Failed to load config data:', error);
        }

        setIsLoading(false);
      };

      loadData();
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []); // Only run on mount

    // Handle session selection
    const handleSelectSession = useCallback(
      (sessionId: string) => {
        navigate(`/pair?resumeSessionId=${sessionId}`, {
          state: { disableAnimation: true },
        });
      },
      [navigate]
    );

    // Handle view all sessions
    const handleViewAll = useCallback(() => {
      navigate('/sessions');
    }, [navigate]);

    // Handle extensions click
    const handleExtensionsClick = useCallback(() => {
      navigate('/extensions');
    }, [navigate]);

    if (isLoading) {
      return (
        <div className="w-full h-full flex flex-col items-center justify-center px-6 py-8">
          <div className="animate-pulse flex flex-col items-center gap-6 w-full max-w-6xl">
            {/* Title skeleton */}
            <div className="h-10 bg-background-subtle rounded-lg w-48" />
            {/* Status bar skeleton */}
            <div className="h-12 bg-background-subtle rounded-xl w-full max-w-lg" />
            {/* Cards skeleton */}
            <div className="w-full mt-4">
              <div className="h-6 bg-background-subtle rounded w-32 mb-4" />
              <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
                {[0, 1, 2, 3].map((i) => (
                  <div key={i} className="h-28 bg-background-subtle rounded-xl" />
                ))}
              </div>
            </div>
          </div>
        </div>
      );
    }

    return (
      <div className="w-full h-full flex flex-col items-center justify-center px-6 py-8 gap-8">
        {/* Title - 响应式字号 */}
        <h1 className="text-2xl sm:text-3xl md:text-4xl font-light text-text-default tracking-wide">
          {t('title')}
        </h1>

        {/* Context status bar */}
        <ContextStatusBar
          workingDir={workingDir}
          extensionCount={enabledExtensionsCount}
          hasMemory={hasMemory}
          onExtensionsClick={handleExtensionsClick}
        />

        {/* Recent sessions or empty state */}
        <div className="flex-1 w-full flex flex-col justify-center">
          {recentSessions.length > 0 ? (
            <RecentSessionsSection
              sessions={recentSessions}
              onSelectSession={handleSelectSession}
              onViewAll={handleViewAll}
            />
          ) : (
            <EmptyState />
          )}
        </div>

        {/* Bottom hint */}
        <p className="text-sm text-text-muted">
          {t('hint')}
        </p>
      </div>
    );
  }
);

export default NewChatWelcome;
