import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useLocation } from 'react-router-dom';
import {
  listSchedules,
  createSchedule,
  deleteSchedule,
  pauseSchedule,
  unpauseSchedule,
  updateSchedule,
  killRunningJob,
  inspectRunningJob,
  ScheduledJob,
} from '../../schedule';
import { ScrollArea } from '../ui/scroll-area';
import { Card } from '../ui/card';
import { Button } from '../ui/button';
import { TrashIcon } from '../icons/TrashIcon';
import { Plus, RefreshCw, Pause, Play, Edit, Square, Eye, CircleDotDashed } from 'lucide-react';
import { NewSchedulePayload, ScheduleModal } from './ScheduleModal';
import ScheduleDetailView from './ScheduleDetailView';
import { toastError, toastSuccess } from '../../toasts';
import cronstrue from 'cronstrue';
import { formatToLocalDateWithTimezone } from '../../utils/date';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ViewOptions } from '../../utils/navigationUtils';

interface SchedulesViewProps {
  onClose?: () => void;
}

const ScheduleCard: React.FC<{
  job: ScheduledJob;
  onNavigateToDetail: (id: string) => void;
  onEdit: (job: ScheduledJob) => void;
  onPause: (id: string) => void;
  onUnpause: (id: string) => void;
  onKill: (id: string) => void;
  onInspect: (id: string) => void;
  onDelete: (id: string) => void;
  actionInProgress: boolean;
  t: (key: string, options?: Record<string, unknown>) => string;
}> = ({
  job,
  onNavigateToDetail,
  onEdit,
  onPause,
  onUnpause,
  onKill,
  onInspect,
  onDelete,
  actionInProgress,
  t,
}) => {
  let readableCron: string;
  try {
    readableCron = cronstrue.toString(job.cron);
  } catch {
    readableCron = job.cron;
  }

  const formattedLastRun = formatToLocalDateWithTimezone(job.last_run);

  return (
    <Card
      className="py-2 px-4 mb-2 bg-background-default border-none hover:bg-background-muted cursor-pointer transition-all duration-150"
      onClick={() => onNavigateToDetail(job.id)}
    >
      <div className="flex justify-between items-start gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 mb-1">
            <h3 className="text-base truncate max-w-[50vw]" title={job.id}>
              {job.id}
            </h3>
            {job.currently_running && (
              <span className="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-300">
                <span className="inline-block w-2 h-2 bg-green-500 rounded-full mr-1 animate-pulse"></span>
                {t('card.running')}
              </span>
            )}
            {job.paused && (
              <span className="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-300">
                <Pause className="w-3 h-3 mr-1" />
                {t('card.paused')}
              </span>
            )}
          </div>
          <p className="text-text-muted text-sm mb-2 line-clamp-2" title={readableCron}>
            {readableCron}
          </p>
          <div className="flex items-center text-xs text-text-muted">
            <span>{t('lastRunLabel')} {formattedLastRun}</span>
          </div>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          {!job.currently_running && (
            <>
              <Button
                onClick={(e) => {
                  e.stopPropagation();
                  onEdit(job);
                }}
                disabled={actionInProgress}
                variant="outline"
                size="sm"
                className="h-8"
              >
                <Edit className="w-4 h-4 mr-1" />
                {t('edit')}
              </Button>
              <Button
                onClick={(e) => {
                  e.stopPropagation();
                  if (job.paused) {
                    onUnpause(job.id);
                  } else {
                    onPause(job.id);
                  }
                }}
                disabled={actionInProgress}
                variant="outline"
                size="sm"
                className="h-8"
              >
                              {job.paused ? (
                  <>
                    <Play className="w-4 h-4 mr-1" />
                    {t('resume')}
                  </>
                ) : (
                  <>
                    <Pause className="w-4 h-4 mr-1" />
                    {t('pause')}
                  </>
                )}
              </Button>
            </>
          )}
          {job.currently_running && (
            <>
              <Button
                onClick={(e) => {
                  e.stopPropagation();
                  onInspect(job.id);
                }}
                disabled={actionInProgress}
                variant="outline"
                size="sm"
                className="h-8"
              >
                <Eye className="w-4 h-4 mr-1" />
                {t('inspect')}
              </Button>
              <Button
                onClick={(e) => {
                  e.stopPropagation();
                  onKill(job.id);
                }}
                disabled={actionInProgress}
                variant="outline"
                size="sm"
                className="h-8"
              >
                <Square className="w-4 h-4 mr-1" />
                {t('kill')}
              </Button>
            </>
          )}
          <Button
            onClick={(e) => {
              e.stopPropagation();
              onDelete(job.id);
            }}
            disabled={actionInProgress}
            variant="ghost"
            size="sm"
            className="h-8 text-red-500 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20"
          >
            <TrashIcon className="w-4 h-4" />
          </Button>
        </div>
      </div>
    </Card>
  );
};

const SchedulesView: React.FC<SchedulesViewProps> = ({ onClose: _onClose }) => {
  const { t } = useTranslation('schedules');
  const { t: tCommon } = useTranslation('common');
  const location = useLocation();
  const [schedules, setSchedules] = useState<ScheduledJob[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [apiError, setApiError] = useState<string | null>(null);
  const [submitApiError, setSubmitApiError] = useState<string | null>(null);
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [editingSchedule, setEditingSchedule] = useState<ScheduledJob | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [pendingDeepLink, setPendingDeepLink] = useState<string | null>(null);
  const [actionsInProgress, setActionsInProgress] = useState<Set<string>>(new Set());
  const [viewingScheduleId, setViewingScheduleId] = useState<string | null>(null);

  const fetchSchedules = useCallback(async () => {
    setIsLoading(true);
    setApiError(null);
    try {
      const fetchedSchedules = await listSchedules();
      setSchedules(fetchedSchedules);
    } catch (error) {
      console.error('Failed to fetch schedules:', error);
      setApiError(
        error instanceof Error
          ? error.message
          : t('errors.unknownFetch')
      );
    } finally {
      setIsLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (viewingScheduleId === null) {
      fetchSchedules();

      const locationState = location.state as ViewOptions | null;
      if (locationState?.pendingScheduleDeepLink) {
        setPendingDeepLink(locationState.pendingScheduleDeepLink);
        setIsModalOpen(true);
        window.history.replaceState({}, document.title);
      }
    }
  }, [viewingScheduleId, location.state, fetchSchedules]);

  useEffect(() => {
    if (viewingScheduleId !== null || actionsInProgress.size > 0) return;

    const intervalId = setInterval(() => {
      if (viewingScheduleId === null && !isRefreshing && !isLoading && !isSubmitting) {
        fetchSchedules();
      }
    }, 15000);

    return () => clearInterval(intervalId);
  }, [viewingScheduleId, isRefreshing, isLoading, isSubmitting, actionsInProgress.size, fetchSchedules]);

  const handleRefresh = async () => {
    setIsRefreshing(true);
    try {
      await fetchSchedules();
    } finally {
      setIsRefreshing(false);
    }
  };

  const handleModalSubmit = async (payload: NewSchedulePayload | string) => {
    setIsSubmitting(true);
    setSubmitApiError(null);
    try {
      if (editingSchedule) {
        await updateSchedule(editingSchedule.id, payload as string);
        toastSuccess({
          title: t('toasts.scheduleUpdated'),
          msg: t('toasts.scheduleUpdatedMsg', { id: editingSchedule.id }),
        });
      } else {
        await createSchedule(payload as NewSchedulePayload);
      }
      await fetchSchedules();
      setIsModalOpen(false);
      setEditingSchedule(null);
    } catch (error) {
      console.error('Failed to save schedule:', error);
      setSubmitApiError(error instanceof Error ? error.message : t('errors.unknownSave'));
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteSchedule = async (id: string) => {
    if (!window.confirm(t('deleteConfirmMessage', { id }))) return;

    setActionsInProgress((prev) => new Set(prev).add(id));
    if (viewingScheduleId === id) setViewingScheduleId(null);
    setApiError(null);

    try {
      await deleteSchedule(id);
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to delete schedule "${id}":`, error);
      setApiError(error instanceof Error ? error.message : t('errors.unknownDelete', { id }));
    } finally {
      setActionsInProgress((prev) => {
        const newSet = new Set(prev);
        newSet.delete(id);
        return newSet;
      });
    }
  };

  const handlePauseSchedule = async (id: string) => {
    setActionsInProgress((prev) => new Set(prev).add(id));
    setApiError(null);

    try {
      await pauseSchedule(id);
      toastSuccess({
        title: t('toasts.schedulePaused'),
        msg: t('toasts.schedulePausedMsg', { id }),
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to pause schedule "${id}":`, error);
      const errorMsg = error instanceof Error ? error.message : t('errors.unknownPause', { id });
      setApiError(errorMsg);
      toastError({
        title: t('toasts.pauseError'),
        msg: errorMsg,
      });
    } finally {
      setActionsInProgress((prev) => {
        const newSet = new Set(prev);
        newSet.delete(id);
        return newSet;
      });
    }
  };

  const handleUnpauseSchedule = async (id: string) => {
    setActionsInProgress((prev) => new Set(prev).add(id));
    setApiError(null);

    try {
      await unpauseSchedule(id);
      toastSuccess({
        title: t('toasts.scheduleUnpaused'),
        msg: t('toasts.scheduleUnpausedMsg', { id }),
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to unpause schedule "${id}":`, error);
      const errorMsg = error instanceof Error ? error.message : t('errors.unknownUnpause', { id });
      setApiError(errorMsg);
      toastError({
        title: t('toasts.unpauseError'),
        msg: errorMsg,
      });
    } finally {
      setActionsInProgress((prev) => {
        const newSet = new Set(prev);
        newSet.delete(id);
        return newSet;
      });
    }
  };

  const handleKillRunningJob = async (id: string) => {
    setActionsInProgress((prev) => new Set(prev).add(id));
    setApiError(null);

    try {
      const result = await killRunningJob(id);
      toastSuccess({
        title: t('toasts.jobKilled'),
        msg: result.message,
      });
      await fetchSchedules();
    } catch (error) {
      console.error(`Failed to kill running job "${id}":`, error);
      const errorMsg =
        error instanceof Error ? error.message : t('errors.unknownKill', { id });
      setApiError(errorMsg);
      toastError({
        title: t('toasts.killError'),
        msg: errorMsg,
      });
    } finally {
      setActionsInProgress((prev) => {
        const newSet = new Set(prev);
        newSet.delete(id);
        return newSet;
      });
    }
  };

  const handleInspectRunningJob = async (id: string) => {
    setActionsInProgress((prev) => new Set(prev).add(id));
    setApiError(null);

    try {
      const result = await inspectRunningJob(id);
      if (result.sessionId) {
        const duration = result.runningDurationSeconds
          ? `${Math.floor(result.runningDurationSeconds / 60)}m ${result.runningDurationSeconds % 60}s`
          : t('errors.unknownDuration');
        toastSuccess({
          title: t('toasts.jobInspection'),
          msg: t('toasts.sessionInfo', { sessionId: result.sessionId, duration }),
        });
      } else {
        toastSuccess({
          title: t('toasts.jobInspection'),
          msg: t('toasts.noDetailInfo'),
        });
      }
    } catch (error) {
      console.error(`Failed to inspect running job "${id}":`, error);
      const errorMsg =
        error instanceof Error ? error.message : t('errors.unknownInspect', { id });
      setApiError(errorMsg);
      toastError({
        title: t('toasts.inspectError'),
        msg: errorMsg,
      });
    } finally {
      setActionsInProgress((prev) => {
        const newSet = new Set(prev);
        newSet.delete(id);
        return newSet;
      });
    }
  };

  if (viewingScheduleId) {
    return (
      <ScheduleDetailView
        scheduleId={viewingScheduleId}
        onNavigateBack={() => setViewingScheduleId(null)}
      />
    );
  }

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0">
          <div className="bg-background-default px-8 pb-8 pt-16">
            <div className="flex flex-col page-transition">
              <div className="flex justify-between items-center mb-1">
                <h1 className="text-4xl font-light">{t('title')}</h1>
                <div className="flex gap-2">
                  <Button
                    onClick={handleRefresh}
                    disabled={isRefreshing || isLoading}
                    variant="outline"
                    size="sm"
                    className="flex items-center gap-2"
                  >
                    <RefreshCw className={`h-4 w-4 ${isRefreshing ? 'animate-spin' : ''}`} />
                    {isRefreshing ? tCommon('loading') : tCommon('refresh')}
                  </Button>
                  <Button
                    onClick={() => {
                      setSubmitApiError(null);
                      setIsModalOpen(true);
                    }}
                    size="sm"
                    className="flex items-center gap-2"
                  >
                    <Plus className="h-4 w-4" />
                    {tCommon('add')}
                  </Button>
                </div>
              </div>
              <p className="text-sm text-text-muted mb-1">
                {t('description')}
              </p>
            </div>
          </div>

          <div className="flex-1 min-h-0 relative px-8">
            <ScrollArea className="h-full">
              <div className="h-full relative">
                {apiError && (
                  <div className="mb-4 p-4 bg-background-error border border-border-error rounded-md">
                    <p className="text-text-error text-sm">Error: {apiError}</p>
                  </div>
                )}

                {isLoading && schedules.length === 0 && (
                  <div className="flex justify-center items-center py-12">
                    <div className="animate-spin rounded-full h-8 w-8 border-t-2 border-b-2 border-text-default"></div>
                  </div>
                )}

                {!isLoading && !apiError && schedules.length === 0 && (
                  <div className="flex flex-col pt-4 pb-12">
                    <CircleDotDashed className="h-5 w-5 text-text-muted mb-3.5" />
                    <p className="text-base text-text-muted font-light mb-2">{t('noSchedules')}</p>
                  </div>
                )}

                {!isLoading && schedules.length > 0 && (
                  <div className="space-y-2 pb-8">
                    {schedules.map((job) => (
                      <ScheduleCard
                        key={job.id}
                        job={job}
                        onNavigateToDetail={setViewingScheduleId}
                        onEdit={(schedule) => {
                          setEditingSchedule(schedule);
                          setSubmitApiError(null);
                          setIsModalOpen(true);
                        }}
                        onPause={handlePauseSchedule}
                        onUnpause={handleUnpauseSchedule}
                        onKill={handleKillRunningJob}
                        onInspect={handleInspectRunningJob}
                        onDelete={handleDeleteSchedule}
                        actionInProgress={actionsInProgress.has(job.id) || isSubmitting}
                        t={t}
                      />
                    ))}
                  </div>
                )}
              </div>
            </ScrollArea>
          </div>
        </div>
      </MainPanelLayout>

      <ScheduleModal
        isOpen={isModalOpen}
        onClose={() => {
          setIsModalOpen(false);
          setEditingSchedule(null);
          setSubmitApiError(null);
          setPendingDeepLink(null);
        }}
        onSubmit={handleModalSubmit}
        schedule={editingSchedule}
        isLoadingExternally={isSubmitting}
        apiErrorExternally={submitApiError}
        initialDeepLink={pendingDeepLink}
      />
    </>
  );
};

export default SchedulesView;
