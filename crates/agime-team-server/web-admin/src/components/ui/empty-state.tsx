import * as React from 'react';
import { cn } from '../../utils';

interface EmptyStateProps {
  icon?: React.ReactNode;
  message: string;
  action?: React.ReactNode;
  className?: string;
}

export function EmptyState({ icon, message, action, className }: EmptyStateProps) {
  return (
    <div className={cn('flex flex-col items-center justify-center py-8 text-muted-foreground', className)}>
      <span className="text-lg">{icon ?? '◇'}</span>
      <p className="text-sm mt-1">{message}</p>
      {action && <div className="mt-3">{action}</div>}
    </div>
  );
}
