import React, { memo } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Check } from 'lucide-react';
import { cn } from '../../../../utils';

export interface Step {
  id: number;
  title: string;
  description?: string;
}

interface StepIndicatorProps {
  steps: Step[];
  currentStep: number;
  className?: string;
}

export const StepIndicator = memo(function StepIndicator({
  steps,
  currentStep,
  className,
}: StepIndicatorProps) {
  return (
    <div className={cn('flex items-center justify-center w-full', className)}>
      {steps.map((step, index) => {
        const isCompleted = currentStep > step.id;
        const isCurrent = currentStep === step.id;
        const isLast = index === steps.length - 1;

        return (
          <React.Fragment key={step.id}>
            {/* Step circle and label */}
            <div className="flex flex-col items-center">
              <div
                className={cn(
                  'w-8 h-8 rounded-full flex items-center justify-center text-xs font-semibold transition-all duration-300',
                  isCompleted
                    ? 'bg-gradient-to-br from-block-teal to-block-teal/80 text-white shadow-md shadow-block-teal/30'
                    : isCurrent
                    ? 'bg-gradient-to-br from-block-teal to-block-orange text-white shadow-lg shadow-block-teal/40 ring-4 ring-block-teal/20'
                    : 'bg-background-muted text-text-muted border border-border-default'
                )}
              >
                {isCompleted ? (
                  <Check className="w-4 h-4" strokeWidth={3} />
                ) : (
                  step.id
                )}
              </div>
              <span
                className={cn(
                  'mt-2 text-xs font-medium text-center whitespace-nowrap transition-colors duration-200',
                  isCurrent
                    ? 'text-block-teal font-semibold'
                    : isCompleted
                    ? 'text-block-teal/80'
                    : 'text-text-muted'
                )}
              >
                {step.title}
              </span>
            </div>

            {/* Connector line */}
            {!isLast && (
              <div className="flex-1 h-0.5 mx-3 mt-[-16px] relative overflow-hidden rounded-full">
                <div className={cn(
                  'absolute inset-0 transition-all duration-500',
                  isCompleted
                    ? 'bg-gradient-to-r from-block-teal to-block-teal/60'
                    : 'bg-border-default'
                )} />
                {isCurrent && (
                  <div className="absolute inset-y-0 left-0 w-1/2 bg-gradient-to-r from-block-teal to-transparent animate-pulse" />
                )}
              </div>
            )}
          </React.Fragment>
        );
      })}
    </div>
  );
});

// Predefined steps for Quick Setup - function that returns translated steps
export const getQuickSetupSteps = (t: TFunction<'settings'>): Step[] => [
  { id: 1, title: t('quickSetup.steps.selectProvider'), description: t('quickSetup.steps.selectProviderDesc') },
  { id: 2, title: t('quickSetup.steps.credentials'), description: t('quickSetup.steps.credentialsDesc') },
  { id: 3, title: t('quickSetup.steps.selectModel'), description: t('quickSetup.steps.selectModelDesc') },
  { id: 4, title: t('quickSetup.steps.confirmCapabilities'), description: t('quickSetup.steps.confirmCapabilitiesDesc') },
];

// Custom hook to get translated steps
export const useQuickSetupSteps = (): Step[] => {
  const { t } = useTranslation('settings');
  return getQuickSetupSteps(t);
};

export default StepIndicator;
