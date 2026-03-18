import { useTranslation } from 'react-i18next';
import type { MissionStep, StepStatus } from '../../api/mission';
import { localizeMissionError } from '../../utils/missionError';

function formatDuration(startedAt?: string, completedAt?: string): string | null {
  if (!startedAt) return null;
  const start = new Date(startedAt).getTime();
  const end = completedAt ? new Date(completedAt).getTime() : Date.now();
  const sec = Math.round((end - start) / 1000);
  if (sec < 60) return `${sec}s`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m${sec % 60}s`;
  return `${Math.floor(sec / 3600)}h${Math.floor((sec % 3600) / 60)}m`;
}

interface MissionStepListProps {
  steps: MissionStep[];
  currentStep?: number;
  selectedStep?: number;
  onSelectStep?: (stepIndex: number) => void;
  onApprove?: (stepIndex: number) => void;
  onReject?: (stepIndex: number) => void;
  onSkip?: (stepIndex: number) => void;
}

const dotStyle: Record<StepStatus, string> = {
  pending: 'border border-muted-foreground/30 bg-transparent',
  awaiting_approval: 'border border-[hsl(var(--status-warning-text))/0.42] bg-[hsl(var(--status-warning-bg))/0.92]',
  running: 'bg-foreground/70 animate-pulse',
  completed: 'bg-muted-foreground/50',
  failed: 'bg-status-error-text/75',
  skipped: 'bg-muted-foreground/20',
};

function readableStepStatus(status: StepStatus, t: ReturnType<typeof useTranslation>['t']): string {
  switch (status) {
    case 'awaiting_approval':
      return t('mission.awaitingApproval');
    case 'skipped':
      return t('mission.skipped');
    default:
      return t(`mission.${status}`, status);
  }
}

export function MissionStepList({
  steps, currentStep, selectedStep, onSelectStep, onApprove, onReject, onSkip,
}: MissionStepListProps) {
  const { t } = useTranslation();

  if (steps.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-8 text-muted-foreground/65">
        <span className="text-lg">◇</span>
        <span className="text-xs mt-1">{t('mission.planning', 'Planning...')}</span>
      </div>
    );
  }

  return (
    <div className="py-1">
      {steps.map((step, i) => {
        const isCurrent = currentStep === step.index;
        const isSelected = selectedStep === step.index;
        const dur = formatDuration(step.started_at, step.completed_at);
        const isLast = i === steps.length - 1;

        return (
          <div
            key={step.index}
            onClick={() => onSelectStep?.(step.index)}
            className="flex gap-3 cursor-pointer group"
          >
            {/* Timeline track */}
            <div className="flex flex-col items-center w-4 shrink-0">
              <div className={`w-2 h-2 rounded-full mt-1.5 shrink-0 ${dotStyle[step.status]}`} />
              {!isLast && <div className="w-px flex-1 bg-border/50 my-1" />}
            </div>

            {/* Content */}
            <div className={`flex-1 min-w-0 rounded-2xl border px-3 py-3 ${isCurrent || isSelected ? 'border-[hsl(var(--status-info-text))/0.2] bg-status-info-bg/50 shadow-[0_16px_36px_-30px_rgba(35,64,138,0.35)]' : 'border-border/50 bg-background/76 opacity-72 group-hover:opacity-100'} transition-all`}>
              <div className="flex items-center gap-2">
                <span className={`text-sm truncate ${isCurrent ? 'font-semibold' : 'font-medium'}`}>
                  {step.title}
                </span>
                {step.is_checkpoint && (
                  <span className="rounded-full border border-muted-foreground/20 px-2 py-0.5 text-[11px] text-muted-foreground/75">CP</span>
                )}
                <span className="ml-auto flex shrink-0 items-center gap-2 text-caption text-muted-foreground/70">
                  {dur && <span>{dur}</span>}
                  {step.retry_count > 0 && <span>R{step.retry_count}</span>}
                </span>
              </div>
              <div className="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-muted-foreground/68">
                <span className="rounded-full border border-border/45 bg-muted/18 px-2 py-0.5">
                  {readableStepStatus(step.status, t)}
                </span>
                {step.supervisor_state && (
                  <span className="rounded-full border border-border/45 bg-muted/18 px-2 py-0.5">
                    {step.supervisor_state}
                  </span>
                )}
              </div>

              {step.error_message && (
                <p className="mt-2 line-clamp-2 text-xs leading-5 text-status-error-text/85">
                  {localizeMissionError(step.error_message, t)}
                </p>
              )}

              {!step.error_message && step.current_blocker && (
                <p className="mt-2 line-clamp-2 text-xs leading-5 text-muted-foreground/72">
                  {step.current_blocker}
                </p>
              )}

              {step.status === 'awaiting_approval' && (
                <div className="flex gap-2 mt-2" onClick={e => e.stopPropagation()}>
                  {onApprove && (
                    <button onClick={() => onApprove(step.index)}
                      className="text-xs px-2.5 py-1 rounded-sm bg-foreground text-background hover:opacity-80 transition-opacity">
                      {t('mission.approve')}
                    </button>
                  )}
                  {onReject && (
                    <button onClick={() => onReject(step.index)}
                      className="text-xs px-2.5 py-1 rounded-sm border border-border hover:bg-accent transition-colors">
                      {t('mission.reject')}
                    </button>
                  )}
                  {onSkip && (
                    <button onClick={() => onSkip(step.index)}
                      className="text-xs px-2.5 py-1 rounded-sm border border-border text-muted-foreground hover:bg-accent transition-colors">
                      {t('mission.skip')}
                    </button>
                  )}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}
