import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardContent, CardDescription } from '../ui/card';
import { Greeting } from '../common/Greeting';
import { useNavigate } from 'react-router-dom';
import { Button } from '../ui/button';
import { ChatSmart } from '../icons/';
import { Goose } from '../icons/Goose';
import { Skeleton } from '../ui/skeleton';
import {
  getSessionInsights,
  listSessions,
  Session,
  SessionInsights as ApiSessionInsights,
} from '../../api';
import { resumeSession } from '../../sessions';
import { useNavigation } from '../../hooks/useNavigation';
import { cn } from '../../utils';
import { QuickStarts } from './QuickStarts';

interface SessionInsightsProps {
  onSelectPrompt?: (prompt: string) => void;
}

export function SessionInsights({ onSelectPrompt }: SessionInsightsProps) {
  const [insights, setInsights] = useState<ApiSessionInsights | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recentSessions, setRecentSessions] = useState<Session[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingSessions, setIsLoadingSessions] = useState(true);
  const { t } = useTranslation('sessions');
  const navigate = useNavigate();
  const setView = useNavigation();

  useEffect(() => {
    let loadingTimeout: ReturnType<typeof setTimeout>;

    const loadInsights = async () => {
      try {
        const response = await getSessionInsights({ throwOnError: true });
        setInsights(response.data);
        setError(null);
      } catch (error) {
        console.error('Failed to load insights:', error);
        setError(error instanceof Error ? error.message : 'Failed to load insights');
        setInsights({
          totalSessions: 0,
          totalTokens: 0,
        });
      } finally {
        setIsLoading(false);
      }
    };

    const loadRecentSessions = async () => {
      try {
        const response = await listSessions<true>({ throwOnError: true });
        setRecentSessions(response.data.sessions.slice(0, 3));
      } finally {
        setIsLoadingSessions(false);
      }
    };

    // Set a maximum loading time to prevent infinite skeleton
    loadingTimeout = setTimeout(() => {
      // Only apply fallback if we still don't have insights data
      setInsights((currentInsights) => {
        if (!currentInsights) {
          console.warn('Loading timeout reached, showing fallback content');
          setError('Failed to load insights');
          setIsLoading(false);
          return {
            totalSessions: 0,
            mostActiveDirs: [],
            avgSessionDuration: 0,
            totalTokens: 0,
            recentActivity: [],
          };
        }
        // If we already have insights, just make sure loading is false
        setIsLoading(false);
        return currentInsights;
      });
    }, 10000); // 10 second timeout

    loadInsights();
    loadRecentSessions();

    // Cleanup timeout on unmount
    return () => {
      if (loadingTimeout) {
        window.clearTimeout(loadingTimeout);
      }
    };
  }, []);

  const handleSessionClick = async (session: Session) => {
    try {
      resumeSession(session, setView);
    } catch (error) {
      console.error('Failed to start session:', error);
      navigate('/sessions', {
        state: { selectedSessionId: session.id },
        replace: true,
      });
    }
  };

  const navigateToSessionHistory = () => {
    navigate('/sessions');
  };

  // Format date to show only the date part (without time)
  const formatDateOnly = (dateStr: string) => {
    const date = new Date(dateStr);
    return date
      .toLocaleDateString('en-US', { month: '2-digit', day: '2-digit', year: 'numeric' })
      .replace(/\//g, '/');
  };

  // Render skeleton loader while data is loading
  const renderSkeleton = () => (
    <div className="bg-background-default flex flex-col h-full relative overflow-hidden">
      {/* Header container */}
      <div className="bg-background-default mb-2 relative z-10">
        <div className="px-6 pb-3 pt-12 space-y-2">
          <div className="origin-bottom-left agime-icon-animation">
            <Goose className="size-7" />
          </div>
          <Greeting />
        </div>
      </div>

      {/* Stats containers */}
      <div className="flex flex-col flex-1 px-6 space-y-3 relative z-10">
        {/* Top row with two equal columns */}
        <div className="grid grid-cols-2 gap-3">
          {/* Total Sessions Card Skeleton */}
          <Card className="w-full py-4 px-5 border-none rounded-xl glass">
            <CardContent className="flex flex-col justify-end h-full p-0">
              <div className="flex flex-col justify-end">
                <Skeleton className="h-9 w-16 mb-1" />
                <span className="text-xs text-text-muted">{t('stats.totalSessions')}</span>
              </div>
            </CardContent>
          </Card>

          {/* Total Tokens Card Skeleton */}
          <Card className="w-full py-4 px-5 border-none rounded-xl glass">
            <CardContent className="flex flex-col justify-end h-full p-0">
              <div className="flex flex-col justify-end">
                <Skeleton className="h-9 w-24 mb-1" />
                <span className="text-xs text-text-muted">{t('stats.totalTokens')}</span>
              </div>
            </CardContent>
          </Card>
        </div>

        {/* Recent Chats Card Skeleton */}
        <Card className="w-full py-4 px-5 border-none rounded-xl glass">
          <CardContent className="p-0">
            <div className="flex justify-between items-center mb-2">
              <CardDescription className="mb-0">
                <span className="text-base text-text-default">{t('stats.recentChats')}</span>
              </CardDescription>
              <Button
                variant="ghost"
                size="sm"
                className="text-xs text-text-muted flex items-center gap-1 !px-0 hover:bg-transparent hover:underline hover:text-text-default"
                onClick={navigateToSessionHistory}
              >
                {t('stats.seeAll')}
              </Button>
            </div>
            <div className="space-y-2 min-h-[72px]">
              {/* Skeleton chat items */}
              <div className="flex items-center justify-between py-1 px-2">
                <div className="flex items-center space-x-2">
                  <Skeleton className="h-4 w-4 rounded-sm" />
                  <Skeleton className="h-4 w-48" />
                </div>
                <Skeleton className="h-4 w-16" />
              </div>
              <div className="flex items-center justify-between py-1 px-2">
                <div className="flex items-center space-x-2">
                  <Skeleton className="h-4 w-4 rounded-sm" />
                  <Skeleton className="h-4 w-40" />
                </div>
                <Skeleton className="h-4 w-16" />
              </div>
              <div className="flex items-center justify-between py-1 px-2">
                <div className="flex items-center space-x-2">
                  <Skeleton className="h-4 w-4 rounded-sm" />
                  <Skeleton className="h-4 w-52" />
                </div>
                <Skeleton className="h-4 w-16" />
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Quick Starts Skeleton */}
        <div className="pb-2">
          <Skeleton className="h-4 w-20 mb-2" />
          <div className="grid grid-cols-2 md:grid-cols-4 gap-2">
            {[...Array(8)].map((_, i) => (
              <div key={i} className="p-3 rounded-lg bg-white/5">
                <Skeleton className="h-7 w-7 mb-2 rounded-md" />
                <Skeleton className="h-3 w-20 mb-1" />
                <Skeleton className="h-2 w-24" />
              </div>
            ))}
          </div>
        </div>

        {/* Filler container */}
        <div className="flex-1"></div>
      </div>
    </div>
  );

  // Show skeleton while loading, then show actual content
  if (isLoading) {
    return renderSkeleton();
  }

  return (
    <div className="bg-transparent flex flex-col h-full relative overflow-hidden">
      {/* Header container */}
      <div className="bg-transparent mb-2 relative z-10">
        <div className="px-6 pb-3 pt-12 space-y-2">
          <div className="origin-bottom-left agime-icon-animation">
            <Goose className="size-7" />
          </div>
          <Greeting />
        </div>
      </div>

      {/* Stats containers */}
      <div className="flex flex-col flex-1 px-6 space-y-3 relative z-10">
        {/* Error notice if insights failed to load */}
        {error && (
          <div className="px-4 py-2 bg-orange-50 dark:bg-orange-950/20 border border-orange-200 dark:border-orange-800/30 rounded-xl">
            <div className="flex items-center space-x-2">
              <div className="w-2 h-2 bg-orange-400 rounded-full flex-shrink-0"></div>
              <span className="text-xs text-orange-700 dark:text-orange-300">
                {t('failedToLoadInsights')}
              </span>
            </div>
          </div>
        )}

        {/* Top row with stats cards */}
        <div className="grid grid-cols-2 gap-3">
          {/* Total Sessions Card */}
          <Card className={cn(
            "w-full py-4 px-5 rounded-xl",
            "glass border-none",
            "transition-all duration-300 ease-out",
            "hover:shadow-lg hover:-translate-y-0.5",
            "animate-card-entrance stagger-1"
          )}>
            <CardContent className="flex flex-col justify-end h-full p-0">
              <div className="flex flex-col justify-end">
                <p className="text-4xl font-mono font-light text-gradient-teal">
                  {Math.max(insights?.totalSessions ?? 0, 0)}
                </p>
                <span className="text-xs text-text-muted mt-1">{t('stats.totalSessions')}</span>
              </div>
            </CardContent>
          </Card>

          {/* Total Tokens Card */}
          <Card className={cn(
            "w-full py-4 px-5 rounded-xl",
            "glass border-none",
            "transition-all duration-300 ease-out",
            "hover:shadow-lg hover:-translate-y-0.5",
            "animate-card-entrance stagger-2"
          )}>
            <CardContent className="flex flex-col justify-end h-full p-0">
              <div className="flex flex-col justify-end">
                <p className="text-4xl font-mono font-light text-gradient-teal">
                  {insights?.totalTokens && insights.totalTokens > 0
                    ? `${(insights.totalTokens / 1000000).toFixed(2)}M`
                    : '0.00M'}
                </p>
                <span className="text-xs text-text-muted mt-1">{t('stats.totalTokens')}</span>
              </div>
            </CardContent>
          </Card>
        </div>

        {/* Recent Chats Card */}
        <Card className={cn(
          "w-full py-4 px-5 rounded-xl",
          "glass border-none",
          "animate-card-entrance stagger-3"
        )}>
          <CardContent className="p-0">
            <div className="flex justify-between items-center mb-2">
              <CardDescription className="mb-0">
                <span className="text-base text-text-default font-medium">{t('stats.recentChats')}</span>
              </CardDescription>
              <Button
                variant="ghost"
                size="sm"
                className="text-xs text-text-muted flex items-center gap-1 !px-0 hover:bg-transparent hover:underline hover:text-text-default"
                onClick={navigateToSessionHistory}
              >
                {t('stats.seeAll')}
              </Button>
            </div>
            <div className="space-y-0.5 min-h-[72px] transition-all duration-150">
              {isLoadingSessions ? (
                // Show skeleton while sessions are loading
                <>
                  <div className="flex items-center justify-between py-1 px-2">
                    <div className="flex items-center space-x-2">
                      <Skeleton className="h-4 w-4 rounded-sm" />
                      <Skeleton className="h-4 w-48" />
                    </div>
                    <Skeleton className="h-4 w-16" />
                  </div>
                  <div className="flex items-center justify-between py-1 px-2">
                    <div className="flex items-center space-x-2">
                      <Skeleton className="h-4 w-4 rounded-sm" />
                      <Skeleton className="h-4 w-40" />
                    </div>
                    <Skeleton className="h-4 w-16" />
                  </div>
                  <div className="flex items-center justify-between py-1 px-2">
                    <div className="flex items-center space-x-2">
                      <Skeleton className="h-4 w-4 rounded-sm" />
                      <Skeleton className="h-4 w-52" />
                    </div>
                    <Skeleton className="h-4 w-16" />
                  </div>
                </>
              ) : recentSessions.length > 0 ? (
                recentSessions.map((session) => (
                  <div
                    key={session.id}
                    className={cn(
                      "flex items-center justify-between text-sm py-2 px-2.5 rounded-lg",
                      "hover:bg-block-teal/10 cursor-pointer",
                      "transition-all duration-200",
                      "border-l-2 border-transparent hover:border-block-teal"
                    )}
                    onClick={() => handleSessionClick(session)}
                    role="button"
                    tabIndex={0}
                    onKeyDown={async (e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        await handleSessionClick(session);
                      }
                    }}
                  >
                    <div className="flex items-center space-x-3">
                      <ChatSmart className="h-4 w-4 text-block-teal" />
                      <span className="truncate max-w-[300px]">{session.name}</span>
                    </div>
                    <span className="text-text-muted font-mono font-light text-xs">
                      {formatDateOnly(session.updated_at)}
                    </span>
                  </div>
                ))
              ) : (
                <div className="text-text-muted text-sm py-2">{t('stats.noRecentChats')}</div>
              )}
            </div>
          </CardContent>
        </Card>

        {/* Quick Starts Section */}
        {onSelectPrompt && (
          <div className="animate-card-entrance stagger-4">
            <QuickStarts onSelectPrompt={onSelectPrompt} />
          </div>
        )}

        {/* Filler container - extends to fill remaining space */}
        <div className="flex-1"></div>
      </div>
    </div>
  );
}
