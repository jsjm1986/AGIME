import { useTranslation } from 'react-i18next';
import type { MissionStep, StepStatus } from '../../api/mission';

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
  awaiting_approval: 'border border-yellow-500 bg-yellow-500/20',
  running: 'bg-foreground/70 animate-pulse',
  completed: 'bg-muted-foreground/50',
  failed: 'bg-red-500/70',
  skipped: 'bg-muted-foreground/20',
};

export function MissionStepList({
  steps, currentStep, selectedStep, onSelectStep, onApprove, onReject, onSkip,
}: MissionStepListProps) {
  const { t } = useTranslation();

  if (steps.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-8 text-muted-foreground/40">
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
            <div className={`flex-1 min-w-0 pb-4 ${isCurrent || isSelected ? '' : 'opacity-60 group-hover:opacity-100'} transition-opacity`}>
              <div className="flex items-center gap-2">
                <span className={`text-sm truncate ${isCurrent ? 'font-semibold' : 'font-medium'}`}>
                  {step.title}
                </span>
                {step.is_checkpoint && (
                  <span className="text-[10px] px-1 py-0.5 rounded border border-muted-foreground/20 text-muted-foreground/60">CP</span>
                )}
                <span className="ml-auto flex items-center gap-2 text-[11px] text-muted-foreground/50 shrink-0">
                  {dur && <span>{dur}</span>}
                  {step.retry_count > 0 && <span>R{step.retry_count}</span>}
                </span>
              </div>

              {step.error_message && (
                <p className="text-xs text-red-400/80 mt-0.5 truncate">{step.error_message}</p>
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
