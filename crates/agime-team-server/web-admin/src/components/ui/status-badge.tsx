import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '../../utils';

const statusBadgeVariants = cva(
  'inline-flex items-center rounded-[calc(var(--radius)+3px)] border px-2 py-0.5 text-micro font-medium tracking-[0.02em]',
  {
    variants: {
      status: {
        success: 'border-[hsl(var(--status-success-text))/0.16] bg-status-success-bg text-status-success-text',
        warning: 'border-[hsl(var(--status-warning-text))/0.16] bg-status-warning-bg text-status-warning-text',
        error: 'border-[hsl(var(--status-error-text))/0.16] bg-status-error-bg text-status-error-text',
        info: 'border-[hsl(var(--status-info-text))/0.16] bg-status-info-bg text-status-info-text',
        neutral: 'border-[hsl(var(--status-neutral-text))/0.12] bg-status-neutral-bg text-status-neutral-text',
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
  queued: 'warning',
  running: 'success',
  completed: 'success',
  rejected: 'error',
  failed: 'error',
  cancelled: 'neutral',
};
