import * as React from 'react';
import { Slot } from '@radix-ui/react-slot';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '../../utils';

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-[10px] border border-transparent text-[12px] font-medium tracking-[0.01em] transition-[background-color,border-color,color,box-shadow,transform] duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 disabled:pointer-events-none disabled:opacity-50 active:translate-y-[0.5px]',
  {
    variants: {
      variant: {
        default: 'border-[hsl(var(--primary))/0.9] bg-[hsl(var(--primary))] text-[hsl(var(--primary-foreground))] shadow-none hover:bg-[hsl(var(--primary))/0.9]',
        destructive: 'border-[hsl(var(--destructive))/0.82] bg-[hsl(var(--destructive))] text-[hsl(var(--destructive-foreground))] shadow-none hover:bg-[hsl(var(--destructive))]/0.9',
        outline: 'border-[hsl(var(--border))/0.42] bg-transparent text-[hsl(var(--foreground))] shadow-none hover:bg-[hsl(var(--accent))]/0.22',
        secondary: 'border-transparent bg-[hsl(var(--secondary))/0.52] text-[hsl(var(--secondary-foreground))] shadow-none hover:bg-[hsl(var(--secondary))/0.72]',
        ghost: 'border-transparent bg-transparent text-[hsl(var(--foreground))] shadow-none hover:bg-[hsl(var(--accent))]/0.22 hover:text-[hsl(var(--accent-foreground))]',
        link: 'border-transparent bg-transparent px-0 text-[hsl(var(--primary))] underline-offset-4 hover:underline',
      },
      size: {
        default: 'h-9 px-4 py-2',
        sm: 'h-8 px-3 text-[11px]',
        lg: 'h-10 px-5 text-[12px]',
        icon: 'h-9 w-9',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  }
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, type, ...props }, ref) => {
    const Comp = asChild ? Slot : 'button';
    return (
      <Comp
        className={cn(buttonVariants({ variant, size, className }))}
        ref={ref}
        {...(!asChild ? { type: type ?? 'button' } : {})}
        {...props}
      />
    );
  }
);
Button.displayName = 'Button';

export { Button, buttonVariants };
