import * as React from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '../../utils';

const badgeVariants = cva(
  'inline-flex items-center rounded-[999px] px-2 py-0.5 text-[10px] font-medium tracking-[0.02em] transition-colors focus:outline-none focus:ring-2 focus:ring-[hsl(var(--ring))] focus:ring-offset-2',
  {
    variants: {
      variant: {
        default:
          'bg-[hsl(var(--primary))/0.1] text-[hsl(var(--primary))] hover:bg-[hsl(var(--primary))/0.14]',
        secondary:
          'bg-[hsl(var(--secondary))/0.55] text-[hsl(var(--secondary-foreground))] hover:bg-[hsl(var(--secondary))/0.75]',
        destructive:
          'bg-[hsl(var(--destructive))/0.1] text-[hsl(var(--destructive))] hover:bg-[hsl(var(--destructive))/0.14]',
        outline: 'bg-transparent text-[hsl(var(--foreground))/0.72]',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  }
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge, badgeVariants };
