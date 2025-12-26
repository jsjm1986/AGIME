import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { ScrollArea } from '../ui/scroll-area';
import BackButton from '../ui/BackButton';
import { Card } from '../ui/card';
import {
  getScheduleSessions,
  runScheduleNow,
  pauseSchedule,
  unpauseSchedule,
  updateSchedule,
  listSchedules,
  killRunningJob,
  inspectRunningJob,
  ScheduledJob,
} from '../../schedule';
import SessionHistoryView from '../sessions/SessionHistoryView';
import { ScheduleModal, NewSchedulePayload } from './ScheduleModal';
import { toastError, toastSuccess } from '../../toasts';
import { Loader2, Pause, Play, Edit, Square, Eye } from 'lucide-react';
import cronstrue from 'cronstrue';
import { formatToLocalDateWithTimezone } from '../../utils/date';
import { getSession, Session } from '../../api';

interface ScheduleSessionMeta {
  id: string;
  name: string;
  createdAt: string;
  workingDir?: string;
  scheduleId?: string | null;
  messageCount?: number;
  totalTokens?: number | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  accumulatedTotalTokens?: number | null;
  accumulatedInputTokens?: number | null;
  accumulatedOutputTokens?: number | null;
}

interface ScheduleDetailViewProps {
  scheduleId: string | null;
  onNavigateBack: () => void;
}

const ScheduleDetailView: React.FC<ScheduleDetailViewProps> = ({ scheduleId, onNavigateBack }) => {
  const { t } = useTranslation('schedules');
  const [sessions, setSessions] = useState<ScheduleSessionMeta[]>([]);
  const [isLoadingSessions, setIsLoadingSessions] = useState(false);
  const [sessionsError, setSessionsError] = useState<string | null>(null);

  const [scheduleDetails, setScheduleDetails] = useState<ScheduledJob | null>(null);
  const [isLoadingSchedule, setIsLoadingSchedule] = useState(false);
  const [scheduleError, setScheduleError] = useState<string | null>(null);

  const [isActionLoading, setIsActionLoading] = useState(false);

  const [selectedSession, setSelectedSession] = useState<Session | null>(null);
  const [isLoadingSession, setIsLoadingSession] = useState(false);
  const [sessionError, setSessionError] = useState<string | null>(null);

  const [isModalOpen, setIsModalOpen] = useState(false);

  const fetchSessions = useCallback(async (sId: string) => {
    setIsLoadingSessions(true);
    setSessionsError(null);
    try {
      const data = await getScheduleSessions(sId, 20);
      setSessions(data);
    } catch (err) {
      setSessionsError(err instanceof Error ? err.message : t('detail.failedToFetchSessions'));
    } finally {
      setIsLoadingSessions(false);
    }
  }, [t]);

  const fetchSchedule = useCallback(async (sId: string) => {
    setIsLoadingSchedule(true);
    setScheduleError(null);
    try {
      const allSchedules = await listSchedules();
      const schedule = allSchedules.find((s) => s.id === sId);
      if (schedule) {
        setScheduleDetails(schedule);
      } else {
        setScheduleError(t('detail.scheduleNotFound'));
      }
    } catch (err) {
      setScheduleError(err instanceof Error ? err.message : t('detail.failedToFetchSchedule'));
    } finally {
      setIsLoadingSchedule(false);
    }
  }, [t]);

  useEffect(() => {
    if (scheduleId && !selectedSession) {
      fetchSessions(scheduleId);
      fetchSchedule(scheduleId);
    }
  }, [scheduleId, selectedSession, fetchSchedule, fetchSessions]);

  const handleRunNow = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      const newSessionId = await runScheduleNow(scheduleId);
      if (newSessionId === 'CANCELLED') {
        toastSuccess({ title: t('detail.jobCancelled'), msg: t('detail.jobCancelledMsg') });
      } else {
        toastSuccess({ title: t('detail.scheduleTriggered'), msg: t('detail.newSession', { id: newSessionId }) });
      }
      await fetchSessions(scheduleId);
      await fetchSchedule(scheduleId);
    } catch (err) {
      toastError({
        title: t('detail.runError'),
        msg: err instanceof Error ? err.message : t('detail.failedToTrigger'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handlePauseToggle = async () => {
    if (!scheduleId || !scheduleDetails) return;
    setIsActionLoading(true);
    try {
      if (scheduleDetails.paused) {
        await unpauseSchedule(scheduleId);
        toastSuccess({ title: t('scheduleResumed'), msg: `${scheduleId}` });
      } else {
        await pauseSchedule(scheduleId);
        toastSuccess({ title: t('schedulePaused'), msg: `${scheduleId}` });
      }
      await fetchSchedule(scheduleId);
    } catch (err) {
      toastError({
        title: t('detail.pauseError'),
        msg: err instanceof Error ? err.message : t('detail.operationFailed'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleKill = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      const result = await killRunningJob(scheduleId);
      toastSuccess({ title: t('detail.jobKilled'), msg: result.message });
      await fetchSchedule(scheduleId);
    } catch (err) {
      toastError({
        title: t('detail.killError'),
        msg: err instanceof Error ? err.message : t('detail.failedToKill'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleInspect = async () => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      const result = await inspectRunningJob(scheduleId);
      if (result.sessionId) {
        const duration = result.runningDurationSeconds
          ? `${Math.floor(result.runningDurationSeconds / 60)}m ${result.runningDurationSeconds % 60}s`
          : t('detail.unknownDuration');
        toastSuccess({
          title: t('detail.jobInspection'),
          msg: t('detail.sessionRunning', { id: result.sessionId, duration }),
        });
      } else {
        toastSuccess({ title: t('detail.jobInspection'), msg: t('detail.noDetailInfo') });
      }
    } catch (err) {
      toastError({
        title: t('detail.inspectError'),
        msg: err instanceof Error ? err.message : t('detail.failedToInspect'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const handleModalSubmit = async (payload: NewSchedulePayload | string) => {
    if (!scheduleId) return;
    setIsActionLoading(true);
    try {
      await updateSchedule(scheduleId, payload as string);
      toastSuccess({ title: t('detail.scheduleUpdated'), msg: `${scheduleId}` });
      await fetchSchedule(scheduleId);
      setIsModalOpen(false);
    } catch (err) {
      toastError({
        title: t('detail.updateError'),
        msg: err instanceof Error ? err.message : t('detail.failedToUpdate'),
      });
    } finally {
      setIsActionLoading(false);
    }
  };

  const loadSession = async (sessionId: string) => {
    setIsLoadingSession(true);
    setSessionError(null);
    try {
      const response = await getSession<true>({
        path: { session_id: sessionId },
        throwOnError: true,
      });
      setSelectedSession(response.data);
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('detail.failedToLoadSession');
      setSessionError(msg);
      toastError({ title: t('detail.failedToLoadSession'), msg });
    } finally {
      setIsLoadingSession(false);
    }
  };

  if (selectedSession) {
    return (
      <SessionHistoryView
        session={selectedSession}
        isLoading={isLoadingSession}
        error={sessionError}
        onBack={() => setSelectedSession(null)}
        onRetry={() => loadSession(selectedSession.id)}
        showActionButtons={true}
      />
    );
  }

  if (!scheduleId) {
    return (
      <div className="h-screen w-full flex flex-col items-center justify-center bg-white dark:bg-gray-900 text-text-default p-8">
        <BackButton onClick={onNavigateBack} />
        <h1 className="text-2xl font-medium text-text-prominent mt-4">{t('detail.notFound')}</h1>
        <p className="text-text-subtle mt-2">{t('detail.noIdProvided')}</p>
      </div>
    );
  }

  const readableCron = scheduleDetails
    ? (() => {
        try {
          return cronstrue.toString(scheduleDetails.cron);
        } catch {
          return scheduleDetails.cron;
        }
      })()
    : '';

  return (
    <div className="h-screen w-full flex flex-col bg-background-default text-text-default">
      <div className="px-8 pt-6 pb-4 border-b border-border-subtle flex-shrink-0">
        <BackButton onClick={onNavigateBack} />
        <h1 className="text-4xl font-light mt-1 mb-1 pt-8">{t('detail.title')}</h1>
        <p className="text-sm text-text-muted mb-1">{t('detail.viewingId', { id: scheduleId })}</p>
      </div>

      <ScrollArea className="flex-grow">
        <div className="p-8 space-y-6">
          <section>
            <h2 className="text-xl font-semibold text-text-prominent mb-3">{t('detail.scheduleInfo')}</h2>
            {isLoadingSchedule && (
              <div className="flex items-center text-text-subtle">
                <Loader2 className="mr-2 h-4 w-4 animate-spin" /> {t('detail.loadingSchedule')}
              </div>
            )}
            {scheduleError && (
              <p className="text-text-error text-sm p-3 bg-background-error border border-border-error rounded-md">
                Error: {scheduleError}
              </p>
            )}
            {scheduleDetails && (
              <Card className="p-4 bg-background-card shadow mb-6">
                <div className="space-y-2">
                  <div className="flex flex-col md:flex-row md:items-center justify-between">
                    <h3 className="text-base font-semibold text-text-prominent">
                      {scheduleDetails.id}
                    </h3>
                    <div className="mt-2 md:mt-0 flex items-center gap-2">
                      {scheduleDetails.currently_running && (
                        <div className="text-sm text-green-500 dark:text-green-400 font-semibold flex items-center">
                          <span className="inline-block w-2 h-2 bg-green-500 dark:bg-green-400 rounded-full mr-1 animate-pulse"></span>
                          {t('detail.currentlyRunning')}
                        </div>
                      )}
                      {scheduleDetails.paused && (
                        <div className="text-sm text-orange-500 dark:text-orange-400 font-semibold flex items-center">
                          <Pause className="w-3 h-3 mr-1" />
                          {t('status.paused')}
                        </div>
                      )}
                    </div>
                  </div>
                  <p className="text-sm text-text-default">
                    <span className="font-semibold">{t('detail.schedule')}</span> {readableCron}
                  </p>
                  <p className="text-sm text-text-default">
                    <span className="font-semibold">{t('detail.cronExpression')}</span> {scheduleDetails.cron}
                  </p>
                  <p className="text-sm text-text-default">
                    <span className="font-semibold">{t('detail.recipeSource')}</span> {scheduleDetails.source}
                  </p>
                  <p className="text-sm text-text-default">
                    <span className="font-semibold">{t('lastRun', { time: '' })}</span>{' '}
                    {formatToLocalDateWithTimezone(scheduleDetails.last_run)}
                  </p>
                  {scheduleDetails.currently_running && scheduleDetails.current_session_id && (
                    <p className="text-sm text-text-default">
                      <span className="font-semibold">{t('detail.currentSession')}</span>{' '}
                      {scheduleDetails.current_session_id}
                    </p>
                  )}
                  {scheduleDetails.currently_running && scheduleDetails.process_start_time && (
                    <p className="text-sm text-text-default">
                      <span className="font-semibold">{t('detail.processStarted')}</span>{' '}
                      {formatToLocalDateWithTimezone(scheduleDetails.process_start_time)}
                    </p>
                  )}
                </div>
              </Card>
            )}
          </section>

          <section>
            <h2 className="text-xl font-semibold text-text-prominent mb-3">{t('detail.actions')}</h2>
            <div className="flex flex-col md:flex-row gap-2">
              <Button
                onClick={handleRunNow}
                disabled={isActionLoading || scheduleDetails?.currently_running}
                className="w-full md:w-auto"
              >
                {t('detail.runNow')}
              </Button>

              {scheduleDetails && !scheduleDetails.currently_running && (
                <>
                  <Button
                    onClick={() => setIsModalOpen(true)}
                    variant="outline"
                    className="w-full md:w-auto flex items-center gap-2 text-blue-600 dark:text-blue-400 border-blue-300 dark:border-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/20"
                    disabled={isActionLoading}
                  >
                    <Edit className="w-4 h-4" />
                    {t('detail.editSchedule')}
                  </Button>
                  <Button
                    onClick={handlePauseToggle}
                    variant="outline"
                    className={`w-full md:w-auto flex items-center gap-2 ${
                      scheduleDetails.paused
                        ? 'text-green-600 dark:text-green-400 border-green-300 dark:border-green-600 hover:bg-green-50 dark:hover:bg-green-900/20'
                        : 'text-orange-600 dark:text-orange-400 border-orange-300 dark:border-orange-600 hover:bg-orange-50 dark:hover:bg-orange-900/20'
                    }`}
                    disabled={isActionLoading}
                  >
                    {scheduleDetails.paused ? (
                      <>
                        <Play className="w-4 h-4" />
                        {t('detail.unpauseSchedule')}
                      </>
                    ) : (
                      <>
                        <Pause className="w-4 h-4" />
                        {t('detail.pauseSchedule')}
                      </>
                    )}
                  </Button>
                </>
              )}

              {scheduleDetails?.currently_running && (
                <>
                  <Button
                    onClick={handleInspect}
                    variant="outline"
                    className="w-full md:w-auto flex items-center gap-2 text-blue-600 dark:text-blue-400 border-blue-300 dark:border-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/20"
                    disabled={isActionLoading}
                  >
                    <Eye className="w-4 h-4" />
                    {t('detail.inspectJob')}
                  </Button>
                  <Button
                    onClick={handleKill}
                    variant="outline"
                    className="w-full md:w-auto flex items-center gap-2 text-red-600 dark:text-red-400 border-red-300 dark:border-red-600 hover:bg-red-50 dark:hover:bg-red-900/20"
                    disabled={isActionLoading}
                  >
                    <Square className="w-4 h-4" />
                    {t('detail.killJob')}
                  </Button>
                </>
              )}
            </div>

            {scheduleDetails?.currently_running && (
              <p className="text-sm text-amber-600 dark:text-amber-400 mt-2">
                {t('detail.cannotModifyRunning')}
              </p>
            )}

            {scheduleDetails?.paused && (
              <p className="text-sm text-orange-600 dark:text-orange-400 mt-2">
                {t('detail.pausedHint')}
              </p>
            )}
          </section>

          <section>
            <h2 className="text-xl font-semibold text-text-prominent mb-4">{t('detail.recentSessions')}</h2>
            {isLoadingSessions && <p className="text-text-subtle">{t('detail.loadingSessions')}</p>}
            {sessionsError && (
              <p className="text-text-error text-sm p-3 bg-background-error border border-border-error rounded-md">
                Error: {sessionsError}
              </p>
            )}
            {!isLoadingSessions && sessions.length === 0 && (
              <p className="text-text-subtle text-center py-4">
                {t('detail.noSessions')}
              </p>
            )}

            {sessions.length > 0 && (
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {sessions.map((session) => (
                  <Card
                    key={session.id}
                    className="p-4 bg-background-card shadow cursor-pointer hover:shadow-lg transition-shadow duration-200"
                    onClick={() => loadSession(session.id)}
                  >
                    <h3
                      className="text-sm font-semibold text-text-prominent truncate"
                      title={session.name || session.id}
                    >
                      {session.name || `Session ID: ${session.id}`}
                    </h3>
                    <p className="text-xs text-text-subtle mt-1">
                      {t('detail.created')}{' '}
                      {session.createdAt ? formatToLocalDateWithTimezone(session.createdAt) : 'N/A'}
                    </p>
                    {session.messageCount !== undefined && (
                      <p className="text-xs text-text-subtle mt-1">
                        {t('detail.messages')} {session.messageCount}
                      </p>
                    )}
                    {session.workingDir && (
                      <p
                        className="text-xs text-text-subtle mt-1 truncate"
                        title={session.workingDir}
                      >
                        {t('detail.dir')} {session.workingDir}
                      </p>
                    )}
                    {session.accumulatedTotalTokens !== undefined &&
                      session.accumulatedTotalTokens !== null && (
                        <p className="text-xs text-text-subtle mt-1">
                          {t('detail.tokens')} {session.accumulatedTotalTokens}
                        </p>
                      )}
                    <p className="text-xs text-text-muted mt-1">
                      {t('detail.id')} <span className="font-mono">{session.id}</span>
                    </p>
                  </Card>
                ))}
              </div>
            )}
          </section>
        </div>
      </ScrollArea>

      <ScheduleModal
        isOpen={isModalOpen}
        onClose={() => setIsModalOpen(false)}
        onSubmit={handleModalSubmit}
        schedule={scheduleDetails}
        isLoadingExternally={isActionLoading}
        apiErrorExternally={null}
        initialDeepLink={null}
      />
    </div>
  );
};

export default ScheduleDetailView;
