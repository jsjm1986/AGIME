import { useMemo } from "react";
import { useSearchParams } from "react-router-dom";
import { ArrowLeft, Beaker } from "lucide-react";

import { Button } from "../ui/button";
import { Card, CardContent } from "../ui/card";

import { AutomationLabWorkspace } from "./experiment/AutomationLabWorkspace";
import { ExperimentHome } from "./experiment/ExperimentHome";
import { getExperimentLab, type ExperimentLabId } from "./experiment/labRegistry";

interface ExperimentSectionProps {
  teamId: string;
  canManage: boolean;
}

export function ExperimentSection({ teamId, canManage }: ExperimentSectionProps) {
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedLab = searchParams.get("lab");
  const activeLab = useMemo(() => getExperimentLab(requestedLab), [requestedLab]);

  const handleOpenLab = (labId: ExperimentLabId) => {
    const next = new URLSearchParams(searchParams);
    next.set("lab", labId);
    setSearchParams(next, { replace: true });
  };

  const handleBackToHome = () => {
    const next = new URLSearchParams(searchParams);
    next.delete("lab");
    setSearchParams(next, { replace: true });
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[hsl(var(--ui-line-soft))/0.72] pb-3">
        <div className="min-w-0">
          <div className="inline-flex items-center gap-2 rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.92] px-3 py-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            <Beaker className="h-3.5 w-3.5" />
            Experiment
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-3">
            {activeLab ? (
              <Button variant="outline" size="sm" className="rounded-full" onClick={handleBackToHome}>
                <ArrowLeft className="h-3.5 w-3.5" />
                返回实验室首页
              </Button>
            ) : null}
            <h2 className="text-xl font-semibold tracking-tight text-foreground sm:text-2xl">
              {activeLab ? activeLab.name : "实验室"}
            </h2>
          </div>
          <p className="mt-1.5 max-w-3xl text-sm text-muted-foreground">
            {activeLab ? activeLab.tagline : "每个实验应用只保留一个清晰入口，进入之后再进入真实工作区。"}
          </p>
        </div>
        {activeLab ? (
          <div className="rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.92] px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.1em] text-muted-foreground">
            {activeLab.status}
          </div>
        ) : null}
      </div>

      <div className="min-h-0 flex-1">
        {!activeLab ? (
          <ExperimentHome activeLabId={null} onOpenLab={handleOpenLab} />
        ) : activeLab.id === "automation" ? (
          <AutomationLabWorkspace teamId={teamId} canManage={canManage} />
        ) : (
          <Card className="ui-section-panel">
            <CardContent className="flex min-h-[420px] items-center justify-center text-sm text-muted-foreground">
              当前实验尚未开放。
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  );
}
