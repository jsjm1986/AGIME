import { useTranslation } from 'react-i18next';
import type { MissionStep, StepStatus } from '../../api/mission';

interface MissionStepListProps {
  steps: MissionStep[];
  currentStep?: number;
  selectedStep?: number;
  onSelectStep?: (stepIndex: number) => void;
  onApprove?: (stepIndex: number) => void;
  onReject?: (stepIndex: number) => void;
  onSkip?: (stepIndex: number) => void;
}

const stepStatusIcon: Record<StepStatus, string> = {
  pending: '○',
  awaiting_approval: '⏸',
  running: '▶',
  completed: '✓',
  failed: '✗',
  skipped: '⊘',
};

const stepStatusColor: Record<StepStatus, string> = {
  pending: 'text-muted-foreground',
  awaiting_approval: 'text-yellow-600 dark:text-yellow-400',
  running: 'text-blue-600 dark:text-blue-400',
  completed: 'text-green-600 dark:text-green-400',
  failed: 'text-red-600 dark:text-red-400',
  skipped: 'text-gray-400',
};

export function MissionStepList({
  steps,
  currentStep,
  selectedStep,
  onSelectStep,
  onApprove,
  onReject,
  onSkip,
}: MissionStepListProps) {
  const { t } = useTranslation();

  if (steps.length === 0) {
    return (
      <div className="text-sm text-muted-foreground py-4 text-center">
        {t('mission.planning', 'Planning...')}
      </div>
    );
  }

  return (
    <div className="space-y-1">
      {steps.map((step, i) => {
        const isCurrent = currentStep === step.index;
        const isSelected = selectedStep === step.index;
        return (
          <div
            key={step.index}
            onClick={() => onSelectStep?.(step.index)}
            className={`flex items-start gap-3 p-2 rounded-md transition-colors cursor-pointer hover:bg-accent/50 ${
              isSelected ? 'bg-accent ring-1 ring-primary/30' : isCurrent ? 'bg-accent' : ''
            }`}
          >
            {/* Step indicator */}
            <div className="flex flex-col items-center pt-0.5">
              <span className={`text-base font-mono ${stepStatusColor[step.status]}`}>
                {stepStatusIcon[step.status]}
              </span>
              {i < steps.length - 1 && (
                <div className="w-px h-full min-h-[16px] bg-border mt-1" />
              )}
            </div>

            {/* Step content */}
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium truncate">
                  {step.index + 1}. {step.title}
                </span>
                {step.is_checkpoint && (
                  <span className="text-xs px-1.5 py-0.5 rounded bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300">
                    checkpoint
                  </span>
                )}
              </div>

              {(isCurrent || isSelected) && (
                <p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">
                  {step.description}
                </p>
              )}

              {step.error_message && (
                <p className="text-xs text-red-500 mt-0.5">{step.error_message}</p>
              )}

              {/* Approval actions */}
              {step.status === 'awaiting_approval' && (
                <div className="flex gap-2 mt-2" onClick={e => e.stopPropagation()}>
                  {onApprove && (
                    <button
                      onClick={() => onApprove(step.index)}
                      className="text-xs px-2 py-1 rounded bg-green-600 text-white hover:bg-green-700"
                    >
                      {t('mission.approve')}
                    </button>
                  )}
                  {onReject && (
                    <button
                      onClick={() => onReject(step.index)}
                      className="text-xs px-2 py-1 rounded bg-red-600 text-white hover:bg-red-700"
                    >
                      {t('mission.reject')}
                    </button>
                  )}
                  {onSkip && (
                    <button
                      onClick={() => onSkip(step.index)}
                      className="text-xs px-2 py-1 rounded bg-gray-500 text-white hover:bg-gray-600"
                    >
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
