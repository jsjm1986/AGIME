import { Loader2 } from 'lucide-react';
import { cn } from '../../utils';

interface LoadingStateProps {
  variant?: 'spinner' | 'text';
  message?: string;
  className?: string;
}

export function LoadingState({ variant = 'spinner', message, className }: LoadingStateProps) {
  if (variant === 'text') {
    return (
      <div className={cn('text-center py-8 text-muted-foreground text-sm', className)}>
        {message || 'Loading...'}
      </div>
    );
  }

  return (
    <div className={cn('flex flex-col items-center justify-center py-8', className)}>
      <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      {message && <p className="text-sm text-muted-foreground mt-2">{message}</p>}
    </div>
  );
}
