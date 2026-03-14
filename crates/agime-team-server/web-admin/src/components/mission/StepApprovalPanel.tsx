import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { MissionStep } from '../../api/mission';

interface StepApprovalPanelProps {
  step: MissionStep;
  onApprove: (feedback?: string) => void;
  onReject: (feedback?: string) => void;
  onSkip: () => void;
}

export function StepApprovalPanel({
  step,
  onApprove,
  onReject,
  onSkip,
}: StepApprovalPanelProps) {
  const { t } = useTranslation();
  const [feedback, setFeedback] = useState('');

  if (step.status !== 'awaiting_approval') return null;

  return (
    <div className="rounded-lg border border-[hsl(var(--status-warning-text))/0.18] bg-[hsl(var(--status-warning-bg))/0.76] p-4">
      <div className="flex items-center gap-2 mb-3">
        <span className="text-lg text-status-warning-text">⏸</span>
        <span className="text-sm font-semibold">
          {t('mission.awaitingApproval')}
        </span>
      </div>

      <div className="mb-3">
        <p className="text-sm font-medium">{step.title}</p>
        <p className="text-xs text-muted-foreground mt-1">{step.description}</p>
      </div>

      {/* Feedback input */}
      <div className="mb-3">
        <textarea
          value={feedback}
          onChange={e => setFeedback(e.target.value)}
          rows={2}
          className="w-full rounded-md border px-3 py-2 text-sm bg-background resize-none"
          placeholder={t('mission.feedback')}
        />
      </div>

      {/* Action buttons */}
      <div className="flex gap-2">
        <button
          onClick={() => onApprove(feedback || undefined)}
          className="rounded-md bg-primary px-3 py-1.5 text-sm text-primary-foreground hover:opacity-90"
        >
          {t('mission.approve')}
        </button>
        <button
          onClick={() => onReject(feedback || undefined)}
          className="rounded-md border border-[hsl(var(--status-error-text))/0.18] bg-[hsl(var(--status-error-bg))/0.92] px-3 py-1.5 text-sm text-status-error-text hover:opacity-90"
        >
          {t('mission.reject')}
        </button>
        <button
          onClick={onSkip}
          className="px-3 py-1.5 text-sm rounded-md border hover:bg-accent"
        >
          {t('mission.skip')}
        </button>
      </div>
    </div>
  );
}
