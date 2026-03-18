import { useState, type ReactNode } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { useTranslation } from "react-i18next";

interface MobileWorkspaceShellProps {
  summary?: ReactNode;
  stage?: ReactNode;
  actions?: ReactNode;
  rail?: ReactNode;
  panel?: ReactNode;
  children?: ReactNode;
  quickActions?: ReactNode;
  secondary?: ReactNode;
}

export function MobileWorkspaceShell({
  summary,
  stage,
  actions,
  rail,
  panel,
  children,
  quickActions,
  secondary,
}: MobileWorkspaceShellProps) {
  const { t } = useTranslation();
  const resolvedStage = stage ?? children;
  const resolvedActions = actions ?? quickActions;
  const resolvedPanel = panel ?? secondary;
  const [railOpen, setRailOpen] = useState(false);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 pb-[calc(env(safe-area-inset-bottom,0px)+4px)]">
      {summary}
      {resolvedActions ? (
        <div className="rounded-[18px] border border-border/60 bg-[hsl(var(--ui-surface-panel-muted))/0.38] p-2 shadow-[0_10px_18px_hsl(var(--ui-shadow))/0.03]">
          {resolvedActions}
        </div>
      ) : null}
      {resolvedStage ? (
        <section className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[24px] border border-border/60 bg-[linear-gradient(180deg,hsl(var(--ui-surface-panel))/0.99_0%,hsl(var(--ui-surface-panel))/0.93_100%)] shadow-[0_16px_28px_hsl(var(--ui-shadow))/0.05]">
          {resolvedStage}
        </section>
      ) : null}
      {rail ? (
        <>
          <button
            type="button"
            onClick={() => setRailOpen((prev) => !prev)}
            className="flex min-h-11 items-center justify-between rounded-[18px] border border-border/55 bg-[hsl(var(--ui-surface-panel-muted))/0.22] px-3.5 py-2.5 text-left shadow-[0_8px_18px_hsl(var(--ui-shadow))/0.025] transition-colors hover:bg-[hsl(var(--ui-surface-panel-muted))/0.34]"
          >
            <div className="min-w-0">
              <div className="text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground/78">
                {t('mobile.workspace.contextLabel', '辅助层')}
              </div>
              <div className="mt-0.5 text-[12px] font-semibold tracking-[-0.01em] text-foreground">
                {t('mobile.workspace.moreContext', '更多上下文与资源')}
              </div>
            </div>
            <div className="flex h-8 w-8 items-center justify-center rounded-full border border-border/55 bg-background/75 text-muted-foreground">
              {railOpen ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </div>
          </button>
          {railOpen ? (
            <div className="rounded-[20px] border border-border/55 bg-[hsl(var(--ui-surface-panel-muted))/0.26] shadow-[0_10px_18px_hsl(var(--ui-shadow))/0.025]">
              {rail}
            </div>
          ) : null}
        </>
      ) : null}
      {resolvedPanel}
    </div>
  );
}
