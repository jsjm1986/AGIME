import * as React from 'react';
import * as SwitchPrimitives from '@radix-ui/react-switch';
import { cn } from '../../utils';
import { useIsMobile } from '../../hooks/use-mobile';
import { isWeb } from '../../platform';

export const Switch = React.forwardRef<
  React.ElementRef<typeof SwitchPrimitives.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root> & {
    variant?: 'default' | 'mono';
  }
>(({ className, variant = 'default', style, ...props }, ref) => {
  const isMobile = useIsMobile();
  const isMobileWeb = isWeb && isMobile;

  // Use inline styles for mobile web to override any CSS min-height/min-width rules
  const rootStyle: React.CSSProperties | undefined = isMobileWeb
    ? {
        height: '31px',
        width: '51px',
        minHeight: 'unset',
        minWidth: 'unset',
        ...style,
      }
    : style;

  const thumbStyle: React.CSSProperties | undefined = isMobileWeb
    ? {
        height: '27px',
        width: '27px',
        minHeight: 'unset',
        minWidth: 'unset',
      }
    : undefined;

  return (
    <SwitchPrimitives.Root
      className={cn(
        // Base styles
        'peer inline-flex shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50',
        // Size - larger on mobile (fallback, inline styles will override on web)
        isMobile ? 'h-[31px] w-[51px]' : 'h-[16px] w-[28px]',
        // Colors based on variant
        variant === 'default'
          ? 'data-[state=checked]:bg-background-default data-[state=unchecked]:bg-input'
          : isMobileWeb
            ? 'data-[state=checked]:bg-[#34C759] data-[state=unchecked]:bg-[rgba(120,120,128,0.32)]'
            : 'data-[state=checked]:bg-slate-900 dark:data-[state=checked]:bg-white data-[state=unchecked]:bg-slate-300 dark:data-[state=unchecked]:bg-slate-600',
        className
      )}
      style={rootStyle}
      {...props}
      ref={ref}
    >
      <SwitchPrimitives.Thumb
        className={cn(
          'pointer-events-none block rounded-full shadow-lg ring-0 transition-transform',
          // Size - larger on mobile
          isMobile ? 'h-[27px] w-[27px]' : 'h-3 w-3',
          // Transform distance - larger on mobile
          isMobile
            ? 'data-[state=checked]:translate-x-5 data-[state=unchecked]:translate-x-0'
            : 'data-[state=checked]:translate-x-3 data-[state=unchecked]:translate-x-0',
          // Colors based on variant
          variant === 'default'
            ? 'bg-background-default'
            : isMobileWeb
              ? 'bg-white'
              : 'bg-white dark:data-[state=checked]:bg-black dark:data-[state=unchecked]:bg-white'
        )}
        style={thumbStyle}
      />
    </SwitchPrimitives.Root>
  );
});
Switch.displayName = SwitchPrimitives.Root.displayName;
