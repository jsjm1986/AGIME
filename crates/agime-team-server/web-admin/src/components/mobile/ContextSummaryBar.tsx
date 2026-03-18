import type { ReactNode } from "react";

interface SummaryMetric {
  label: string;
  value: ReactNode;
}

interface ContextSummaryBarProps {
  eyebrow?: string;
  title: string;
  description?: ReactNode;
  badge?: ReactNode;
  metrics?: SummaryMetric[];
  actions?: ReactNode;
}

export function ContextSummaryBar({
  eyebrow,
  title,
  description,
  badge,
  metrics = [],
  actions,
}: ContextSummaryBarProps) {
  return (
    <section className="rounded-[18px] border border-border/60 bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel))/0.99_0%,hsl(var(--ui-surface-panel))/0.94_100%)] px-3 py-2.5 shadow-[0_8px_18px_hsl(var(--ui-shadow))/0.035]">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          {eyebrow ? (
            <div className="text-[8px] font-semibold tracking-[0.14em] text-muted-foreground uppercase">
              {eyebrow}
            </div>
          ) : null}
          <div className="mt-0.5 flex flex-wrap items-center gap-1.5">
            <h2 className="text-[13px] font-semibold tracking-[-0.02em] text-foreground">
              {title}
            </h2>
            {badge}
          </div>
          {description ? (
            <div className="mt-1 line-clamp-1 text-[10px] leading-4 text-muted-foreground">
              {description}
            </div>
          ) : null}
        </div>
        {actions ? <div className="shrink-0">{actions}</div> : null}
      </div>
      {metrics.length > 0 ? (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {metrics.map((metric) => (
            <div
              key={metric.label}
              className="min-w-0 max-w-full rounded-full border border-border/50 bg-[hsl(var(--ui-surface-panel-muted))/0.22] px-2 py-1"
            >
              <div className="text-[8px] uppercase tracking-[0.1em] text-muted-foreground/68">
                {metric.label}
              </div>
              <div className="mt-0.5 truncate text-[10px] font-semibold leading-4 text-foreground">
                {metric.value}
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}
