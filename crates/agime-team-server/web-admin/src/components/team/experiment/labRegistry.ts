import { Cable, type LucideIcon } from "lucide-react";

export type ExperimentLabId = "automation";
export type ExperimentLabStatus = "ready" | "alpha" | "planned";

export interface ExperimentLabDefinition {
  id: ExperimentLabId;
  name: string;
  tagline: string;
  summary: string;
  status: ExperimentLabStatus;
  icon: LucideIcon;
  featuredMetric: string;
  accentClassName: string;
  enabled: boolean;
}

export const EXPERIMENT_LABS: readonly ExperimentLabDefinition[] = [
  {
    id: "automation",
    name: "Agentify｜万物智能",
    tagline: "把多个软件系统接成一个可持续对话的 Agent 应用",
    summary:
      "导入 API 资料，用对话生成可持续对话、可执行、可长期运行的 Agent App，让多个软件系统像一个智能体一样协同工作。",
    status: "ready",
    icon: Cable,
    featuredMetric: "首个可用智能应用",
    accentClassName:
      "border-[hsl(var(--primary))/0.26] bg-[linear-gradient(180deg,hsl(var(--primary))/0.08_0%,transparent_100%)]",
    enabled: true,
  },
];

export function getExperimentLab(labId: string | null | undefined) {
  return EXPERIMENT_LABS.find((item) => item.id === labId) || null;
}
