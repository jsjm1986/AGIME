import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '../../utils';

const statusBadgeVariants = cva(
  'inline-flex items-center rounded-full px-1.5 py-0.5 text-micro font-medium',
  {
    variants: {
      status: {
        success: 'bg-status-success-bg text-status-success-text',
        warning: 'bg-status-warning-bg text-status-warning-text',
        error: 'bg-status-error-bg text-status-error-text',
        info: 'bg-status-info-bg text-status-info-text',
        neutral: 'bg-status-neutral-bg text-status-neutral-text',
      },
    },
    defaultVariants: { status: 'neutral' },
  }
);

export type StatusVariant = NonNullable<VariantProps<typeof statusBadgeVariants>['status']>;

export interface StatusBadgeProps
  extends React.HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof statusBadgeVariants> {}

export function StatusBadge({ className, status, ...props }: StatusBadgeProps) {
  return <span className={cn(statusBadgeVariants({ status }), className)} {...props} />;
}

// --- Status mapping helpers ---

export const MISSION_STATUS_MAP: Record<string, StatusVariant> = {
  draft: 'neutral',
  planning: 'info',
  planned: 'info',
  running: 'success',
  paused: 'warning',
  completed: 'success',
  failed: 'error',
  cancelled: 'neutral',
};

export const AGENT_STATUS_MAP: Record<string, StatusVariant> = {
  idle: 'neutral',
  running: 'success',
  paused: 'warning',
  error: 'error',
};

export const DOC_STATUS_MAP: Record<string, StatusVariant> = {
  draft: 'warning',
  accepted: 'success',
  archived: 'neutral',
  superseded: 'error',
};

export const PORTAL_STATUS_MAP: Record<string, StatusVariant> = {
  draft: 'warning',
  published: 'success',
  archived: 'neutral',
};

export const TASK_STATUS_MAP: Record<string, StatusVariant> = {
  pending: 'neutral',
  approved: 'info',
  running: 'success',
  completed: 'success',
  rejected: 'error',
  failed: 'error',
  cancelled: 'neutral',
};
