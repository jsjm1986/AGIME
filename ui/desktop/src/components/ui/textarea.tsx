import * as React from 'react';

import { cn } from '../../utils';

const Textarea = React.forwardRef<HTMLTextAreaElement, React.ComponentProps<'textarea'>>(
  ({ className, ...props }, ref) => {
    return (
      <textarea
        className={cn(
          'flex min-h-[60px] w-full rounded-md border focus:border-border-strong hover:border-border-strong bg-background-default px-3 py-2 text-base transition-colors placeholder:text-textPlaceholder placeholder:font-light focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50 md:text-sm resize-none',
          className
        )}
        ref={ref}
        {...props}
      />
    );
  }
);
Textarea.displayName = 'Textarea';

export { Textarea };
