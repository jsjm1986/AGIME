import { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { X, Loader2, AlertCircle, Check, Info } from 'lucide-react';
import {
  parseTaskText,
  createFromParse,
  updateTask,
  getTask,
} from '../../services/scheduledTaskClient';
import type {
  ScheduledTaskSummary,
  ScheduledTaskParseResult,
  UpdateTaskRequest,
} from '../../types/scheduledTask';
import { toastError } from '../../toasts';

interface ScheduledTaskModalProps {
  task: ScheduledTaskSummary | null;
  initialParseResult: ScheduledTaskParseResult | null;
  onClose: () => void;
  onCreated: (task: ScheduledTaskSummary) => void;
  onUpdated: (task: ScheduledTaskSummary) => void;
}

export default function ScheduledTaskModal({
  task,
  initialParseResult,
  onClose,
  onCreated,
  onUpdated,
}: ScheduledTaskModalProps) {
  const { t } = useTranslation('scheduledTasks');
  const isEditing = !!task;

  const [naturalLanguage, setNaturalLanguage] = useState('');
  const [title, setTitle] = useState('');
  const [prompt, setPrompt] = useState('');
  const [parseResult, setParseResult] = useState<ScheduledTaskParseResult | null>(initialParseResult);
  const [isParsing, setIsParsing] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isLoadingTask, setIsLoadingTask] = useState(isEditing);
  const parseTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  // Load full task detail when editing
  useEffect(() => {
    if (!isEditing || !task) return;

    let cancelled = false;
    const loadTaskDetail = async () => {
      try {
        const detail = await getTask(task.task_id);
        if (cancelled) return;
        setTitle(detail.title);
        setPrompt(detail.prompt);
      } catch (err) {
        if (cancelled) return;
        toastError({ title: t('errors.unknownFetch'), msg: String(err) });
        onCloseRef.current();
      } finally {
        if (!cancelled) setIsLoadingTask(false);
      }
    };

    loadTaskDetail();
    return () => {
      cancelled = true;
    };
  }, [isEditing, task, t]);

  // Auto-parse when natural language text changes (debounced). Enabled in both
  // create and edit mode: in edit mode an entered schedule description lets the
  // user change the task's timing/frequency (otherwise only title+prompt are
  // editable, which silently drops schedule changes).
  useEffect(() => {
    if (parseTimeoutRef.current) {
      clearTimeout(parseTimeoutRef.current);
    }

    if (naturalLanguage.trim().length < 3) {
      // In edit mode, clearing the NL box means "keep the existing schedule",
      // so don't wipe a previously parsed override unless the user is creating.
      if (!isEditing) setParseResult(null);
      return;
    }

    // Guard against an older in-flight parse resolving after a newer one and
    // clobbering state with stale results.
    let cancelled = false;

    parseTimeoutRef.current = setTimeout(async () => {
      setIsParsing(true);
      try {
        const result = await parseTaskText(naturalLanguage);
        if (cancelled) return;
        setParseResult(result);
        // Auto-fill title and prompt from parse result
        if (result.title) setTitle(result.title);
        if (result.prompt) setPrompt(result.prompt);
      } catch (err) {
        if (cancelled) return;
        console.error('Parse error:', err);
        setParseResult(null);
      } finally {
        if (!cancelled) setIsParsing(false);
      }
    }, 500);

    return () => {
      cancelled = true;
      if (parseTimeoutRef.current) {
        clearTimeout(parseTimeoutRef.current);
      }
    };
  }, [naturalLanguage, isEditing]);

  const handleSubmit = async () => {
    if (isSubmitting) return;

    // Validate
    if (!title.trim()) {
      toastError({ title: t('toasts.createError'), msg: t('modal.titleRequired') });
      return;
    }

    setIsSubmitting(true);
    try {
      if (isEditing && task) {
        // Update existing task. If the user entered a new schedule description
        // (parseResult present in edit mode), carry the re-parsed schedule
        // fields through so timing/frequency changes actually persist; the
        // backend re-validates and recomputes next_fire_at. Without a new
        // parse, only title+prompt change and the existing schedule is kept.
        const updates: UpdateTaskRequest = { title, prompt };
        if (parseResult) {
          updates.task_kind = parseResult.task_kind;
          updates.timezone = parseResult.schedule_spec.timezone;
          if (parseResult.task_kind === 'one_shot') {
            updates.one_shot_at = parseResult.schedule_spec.one_shot_at;
          } else if (parseResult.schedule_spec.schedule_config) {
            updates.schedule_config = parseResult.schedule_spec.schedule_config;
          } else if (parseResult.schedule_spec.cron_expression) {
            updates.cron_expression = parseResult.schedule_spec.cron_expression;
          }
        }
        const updated = await updateTask(task.task_id, updates);
        onUpdated(updated);
      } else {
        // Create from parse result
        if (!parseResult?.ready_to_create) {
          toastError({ title: t('toasts.createError'), msg: t('parse.notReady') });
          return;
        }
        const created = await createFromParse({
          preview: parseResult,
          overrides: { title, prompt },
        });
        onCreated(created);
      }
    } catch (err) {
      toastError({
        title: isEditing ? t('toasts.updateError') : t('toasts.createError'),
        msg: String(err),
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  const getConfidenceColor = (confidence: number) => {
    if (confidence >= 0.8) return 'bg-green-500';
    if (confidence >= 0.5) return 'bg-yellow-500';
    return 'bg-red-500';
  };

  const formatSchedule = () => {
    if (!parseResult?.schedule_spec) return null;
    const { schedule_spec } = parseResult;
    const config = schedule_spec.schedule_config;

    switch (config?.mode) {
      case 'every_minutes':
        return t('modal.everyMinutes', { minutes: config.every_minutes });
      case 'every_hours':
        return t('modal.everyHours', { hours: config.every_hours });
      case 'daily_at':
        return t('modal.dailyAt', { time: config.daily_time });
      case 'weekdays_at':
        return t('modal.weekdaysAt', { time: config.daily_time });
      case 'weekly_on':
        return t('modal.weeklyOn', {
          days: config.weekly_days?.join(', '),
          time: config.daily_time,
        });
      case 'custom':
        return t('modal.customCron');
      default:
        return schedule_spec.cron_expression || schedule_spec.one_shot_at;
    }
  };

  if (isLoadingTask) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
        <div className="bg-background-default rounded-lg shadow-xl p-6 max-w-lg w-full mx-4">
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-8 w-8 animate-spin text-teal-500" />
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-background-default rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[90vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-border-subtle">
          <h2 className="text-lg font-semibold text-text-default">
            {isEditing ? t('modal.editTitle') : t('modal.createTitle')}
          </h2>
          <Button variant="ghost" size="sm" onClick={onClose} className="h-8 w-8 p-0">
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-y-auto p-6 space-y-6">
          {/* Natural Language Input — in edit mode this lets the user change
              the schedule (timing/frequency), which is otherwise locked. */}
          <div className="space-y-2">
            <label className="block text-sm font-medium text-text-default">
              {isEditing ? t('modal.naturalLanguageEdit') : t('modal.naturalLanguage')}
            </label>
            <div className="relative">
              <Input
                value={naturalLanguage}
                onChange={(e) => setNaturalLanguage(e.target.value)}
                placeholder={t('modal.naturalLanguagePlaceholder')}
                className="pr-10"
              />
              {isParsing && (
                <Loader2 className="absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 animate-spin text-text-muted" />
              )}
            </div>
            <p className="text-xs text-text-muted">
              {isEditing ? t('modal.naturalLanguageEditHint') : t('parse.placeholder')}
            </p>
          </div>

          {/* Parse Preview */}
          {parseResult && (
            <div className="bg-background-subtle rounded-lg p-4 space-y-4">
              <div className="flex items-center justify-between">
                <span className="text-sm font-medium text-text-default">{t('modal.previewTitle')}</span>
                <div className="flex items-center gap-2">
                  <div className="w-24 h-2 bg-background-default rounded-full overflow-hidden">
                    <div
                      className={`h-full ${getConfidenceColor(parseResult.confidence)} transition-all duration-300`}
                      style={{ width: `${Math.round(parseResult.confidence * 100)}%` }}
                    />
                  </div>
                  <span className="text-xs text-text-muted">
                    {Math.round(parseResult.confidence * 100)}%
                  </span>
                </div>
              </div>

              {parseResult.warnings.length > 0 && (
                <div className="flex items-start gap-2 text-yellow-600 text-sm">
                  <AlertCircle className="h-4 w-4 mt-0.5 flex-shrink-0" />
                  <div>
                    <p className="font-medium">{t('parse.warnings')}</p>
                    <ul className="list-disc list-inside text-xs mt-1">
                      {parseResult.warnings.map((warning, i) => (
                        <li key={i}>{warning}</li>
                      ))}
                    </ul>
                  </div>
                </div>
              )}

              <div className="grid grid-cols-2 gap-4 text-sm">
                <div>
                  <span className="text-text-muted">{t('parse.title')}:</span>
                  <p className="font-medium text-text-default mt-0.5">{parseResult.title}</p>
                </div>
                <div>
                  <span className="text-text-muted">{t('parse.taskKind')}:</span>
                  <p className="font-medium text-text-default mt-0.5">
                    {parseResult.task_kind === 'one_shot' ? t('parse.oneShot') : t('parse.recurring')}
                  </p>
                </div>
                {formatSchedule() && (
                  <div className="col-span-2">
                    <span className="text-text-muted">{t('modal.scheduleDetails')}:</span>
                    <p className="font-medium text-text-default mt-0.5">{formatSchedule()}</p>
                  </div>
                )}
                <div className="col-span-2">
                  <span className="text-text-muted">{t('modal.timezone')}:</span>
                  <p className="font-medium text-text-default mt-0.5">{parseResult.schedule_spec.timezone}</p>
                </div>
              </div>

              <div className="flex items-center gap-2">
                {parseResult.ready_to_create ? (
                  <>
                    <Check className="h-4 w-4 text-green-500" />
                    <span className="text-sm text-green-500">{t('parse.readyToCreate')}</span>
                  </>
                ) : (
                  <>
                    <Info className="h-4 w-4 text-yellow-500" />
                    <span className="text-sm text-yellow-500">{t('parse.notReady')}</span>
                  </>
                )}
              </div>
            </div>
          )}

          {/* Form Fields */}
          <div className="space-y-4">
            <div className="space-y-2">
              <label className="block text-sm font-medium text-text-default">
                {t('modal.titleLabel')}
              </label>
              <Input
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder={t('modal.titlePlaceholder')}
              />
            </div>

            <div className="space-y-2">
              <label className="block text-sm font-medium text-text-default">
                {t('modal.promptLabel')}
              </label>
              <Textarea
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                placeholder={t('modal.promptPlaceholder')}
                rows={4}
                className="resize-none"
              />
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-end gap-3 px-6 py-4 border-t border-border-subtle">
          <Button variant="outline" onClick={onClose} disabled={isSubmitting}>
            {t('modal.cancel')}
          </Button>
          <Button onClick={handleSubmit} disabled={isSubmitting || (!isEditing && naturalLanguage.trim().length < 3)}>
            {isSubmitting ? (
              <>
                <Loader2 className="h-4 w-4 mr-2 animate-spin" />
                {isEditing ? t('modal.updating') : t('modal.creating')}
              </>
            ) : isEditing ? (
              t('modal.updateTask')
            ) : (
              t('modal.createTask')
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}