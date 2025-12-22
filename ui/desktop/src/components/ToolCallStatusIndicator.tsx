import React from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '../utils';

export type ToolCallStatus = 'pending' | 'loading' | 'success' | 'error';

interface ToolCallStatusIndicatorProps {
  status: ToolCallStatus;
  className?: string;
}

export const ToolCallStatusIndicator: React.FC<ToolCallStatusIndicatorProps> = ({
  status,
  className,
}) => {
  const { t } = useTranslation('chat');

  const getStatusStyles = () => {
    switch (status) {
      case 'success':
        return 'bg-emerald-500';
      case 'error':
        return 'bg-red-500';
      case 'loading':
        return 'bg-cyan-500 animate-pulse';
      case 'pending':
      default:
        return 'bg-gray-400';
    }
  };

  return (
    <div
      className={cn(
        'absolute -top-0.5 -right-0.5 w-2 h-2 rounded-full border border-background-default',
        getStatusStyles(),
        className
      )}
      aria-label={t('toolStatus.ariaLabel', { status: t(`toolStatus.${status}`) })}
    />
  );
};

/**
 * Wrapper component that adds a status indicator to a tool icon
 */
interface ToolIconWithStatusProps {
  ToolIcon: React.ComponentType<{ className?: string }>;
  status: ToolCallStatus;
  className?: string;
}

export const ToolIconWithStatus: React.FC<ToolIconWithStatusProps> = ({
  ToolIcon,
  status,
  className,
}) => {
  return (
    <div className={cn('relative inline-block', className)}>
      <ToolIcon className="w-4 h-4 flex-shrink-0 text-cyan-500 dark:text-cyan-400" />
      <ToolCallStatusIndicator status={status} />
    </div>
  );
};
