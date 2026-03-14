import { ReactNode } from 'react';

interface PageHeaderProps {
  title: string;
  description?: string;
  actions?: ReactNode;
}

export function PageHeader({ title, description, actions }: PageHeaderProps) {
  return (
    <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
      <div className="space-y-2">
        <h1 className="font-display text-[28px] font-semibold tracking-[-0.04em] text-[hsl(var(--foreground))]">{title}</h1>
        {description && (
          <p className="max-w-3xl text-sm leading-6 text-[hsl(var(--muted-foreground))/0.94]">{description}</p>
        )}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2 md:justify-end">{actions}</div>}
    </div>
  );
}
