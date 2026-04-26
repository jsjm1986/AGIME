import { ArrowRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "../../ui/button";
import { Card, CardContent } from "../../ui/card";
import { cn } from "../../../utils";

import {
  EXPERIMENT_LABS,
  type ExperimentLabDefinition,
  type ExperimentLabId,
  localizeExperimentLab,
} from "./labRegistry";

interface ExperimentHomeProps {
  activeLabId: ExperimentLabId | null;
  onOpenLab: (labId: ExperimentLabId) => void;
}

function StatusBadge({ lab }: { lab: ExperimentLabDefinition }) {
  const label =
    lab.status === "ready" ? "Ready" : lab.status === "alpha" ? "Alpha" : "Planned";
  const tone =
    lab.status === "ready"
      ? "bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]"
      : lab.status === "alpha"
        ? "bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]"
        : "bg-[hsl(var(--ui-surface-panel-strong))] text-muted-foreground";
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-2.5 py-1 text-[11px] font-semibold uppercase tracking-[0.08em]",
        tone,
      )}
    >
      {label}
    </span>
  );
}

export function ExperimentHome({ activeLabId, onOpenLab }: ExperimentHomeProps) {
  const { i18n } = useTranslation();
  const labs = EXPERIMENT_LABS
    .filter((lab) => lab.enabled)
    .map((lab) => localizeExperimentLab(lab, i18n.language));
  const singleLab = labs.length === 1;

  return (
    <div
      className={cn(
        "grid gap-4",
        singleLab ? "mx-auto max-w-2xl grid-cols-1" : "md:grid-cols-2 2xl:grid-cols-3",
      )}
    >
        {labs.map((lab) => {
          const Icon = lab.icon;
          const isActive = activeLabId === lab.id;
          return (
            <button
              key={lab.id}
              type="button"
              onClick={() => lab.enabled && onOpenLab(lab.id)}
              disabled={!lab.enabled}
              className={cn(
                "text-left transition-transform",
                lab.enabled ? "hover:-translate-y-0.5" : "cursor-not-allowed opacity-82",
              )}
            >
              <Card
                className={cn(
                  "h-full overflow-hidden rounded-[28px] border shadow-none",
                  lab.accentClassName,
                  isActive && "ring-2 ring-[hsl(var(--primary))/0.22]",
                )}
              >
                <CardContent className="flex h-full flex-col gap-5 px-6 py-6">
                  <div className="flex items-start justify-between gap-3">
                    <div className="inline-flex h-12 w-12 items-center justify-center rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.94]">
                      <Icon className="h-5 w-5 text-[hsl(var(--foreground))]" />
                    </div>
                    <StatusBadge lab={lab} />
                  </div>

                  <div className="space-y-2">
                    <div className="text-xl font-semibold tracking-tight text-foreground sm:text-[22px]">
                      {lab.name}
                    </div>
                    <div className="text-sm font-medium text-[hsl(var(--foreground))/0.82]">
                      {lab.tagline}
                    </div>
                    <p className="line-clamp-3 text-sm leading-7 text-muted-foreground">
                      {lab.summary}
                    </p>
                  </div>

                  <div className="mt-auto flex items-center justify-between gap-3 border-t border-[hsl(var(--ui-line-soft))/0.6] pt-4">
                    <div className="text-xs font-medium text-muted-foreground">
                      {lab.featuredMetric}
                    </div>
                    <Button
                      variant={lab.enabled ? "outline" : "secondary"}
                      size="sm"
                      className="rounded-full px-4"
                      disabled={!lab.enabled}
                    >
                      {lab.enabled ? "Open app" : "Coming soon"}
                      {lab.enabled ? <ArrowRight className="h-3.5 w-3.5" /> : null}
                    </Button>
                  </div>
                </CardContent>
              </Card>
            </button>
          );
        })}
    </div>
  );
}
