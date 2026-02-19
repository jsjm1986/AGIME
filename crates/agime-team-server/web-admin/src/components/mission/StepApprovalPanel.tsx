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
    <div className="border rounded-lg p-4 bg-yellow-50 dark:bg-yellow-950/30 border-yellow-200 dark:border-yellow-800">
      <div className="flex items-center gap-2 mb-3">
        <span className="text-yellow-600 dark:text-yellow-400 text-lg">⏸</span>
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
          className="px-3 py-1.5 text-sm rounded-md bg-green-600 text-white hover:bg-green-700"
        >
          {t('mission.approve')}
        </button>
        <button
          onClick={() => onReject(feedback || undefined)}
          className="px-3 py-1.5 text-sm rounded-md bg-red-600 text-white hover:bg-red-700"
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
