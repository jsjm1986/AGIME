import type { ReactNode } from "react";

interface ManagementRailProps {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
}

export function ManagementRail({
  title,
  description,
  action,
  children,
}: ManagementRailProps) {
  return (
    <section className="rounded-[18px] px-3 py-2.5">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <h3 className="text-[11px] font-semibold tracking-[-0.01em] text-foreground">
            {title}
          </h3>
          {description ? (
            <div className="mt-1 line-clamp-2 text-[10px] leading-4 text-muted-foreground/82">
              {description}
            </div>
          ) : null}
        </div>
        {action ? <div className="shrink-0">{action}</div> : null}
      </div>
      <div className="mt-2">{children}</div>
    </section>
  );
}
