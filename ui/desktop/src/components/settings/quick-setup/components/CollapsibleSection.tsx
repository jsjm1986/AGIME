import React, { useState, useCallback, memo } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { cn } from '../../../../utils';

interface CollapsibleSectionProps {
  title: string;
  icon?: React.ReactNode;
  defaultExpanded?: boolean;
  children: React.ReactNode;
  className?: string;
  badge?: string | number;
}

export const CollapsibleSection = memo(function CollapsibleSection({
  title,
  icon,
  defaultExpanded = true,
  children,
  className,
  badge,
}: CollapsibleSectionProps) {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);

  const toggleExpanded = useCallback(() => {
    setIsExpanded(prev => !prev);
  }, []);

  return (
    <div className={cn('mb-4', className)}>
      <button
        type="button"
        onClick={toggleExpanded}
        className={cn(
          'w-full flex items-center justify-between px-3 py-2.5 rounded-xl',
          'bg-background-muted/50 hover:bg-background-muted',
          'border border-transparent hover:border-block-teal/20',
          'transition-all duration-200 ease-out',
          'focus:outline-none focus-visible:ring-2 focus-visible:ring-block-teal/50'
        )}
      >
        <div className="flex items-center gap-2.5">
          {icon && <span className="text-block-teal">{icon}</span>}
          <span className="text-sm font-semibold text-text-default">
            {title}
          </span>
          {badge !== undefined && (
            <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-block-teal/10 text-block-teal">
              {badge}
            </span>
          )}
        </div>
        <span className={cn(
          'text-text-muted transition-transform duration-200',
          isExpanded && 'rotate-0'
        )}>
          {isExpanded ? (
            <ChevronDown className="w-4 h-4" />
          ) : (
            <ChevronRight className="w-4 h-4" />
          )}
        </span>
      </button>

      <div
        className={cn(
          'overflow-hidden transition-all duration-300 ease-out',
          isExpanded ? 'max-h-[2000px] opacity-100 mt-3' : 'max-h-0 opacity-0'
        )}
      >
        {children}
      </div>
    </div>
  );
});

export default CollapsibleSection;
