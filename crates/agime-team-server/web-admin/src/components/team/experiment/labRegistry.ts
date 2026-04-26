import { Cable, type LucideIcon } from "lucide-react";

export type ExperimentLabId = "automation";
export type ExperimentLabStatus = "ready" | "alpha" | "planned";

export interface ExperimentLabDefinition {
  id: ExperimentLabId;
  name: string;
  nameEn?: string;
  tagline: string;
  taglineEn?: string;
  summary: string;
  summaryEn?: string;
  status: ExperimentLabStatus;
  icon: LucideIcon;
  featuredMetric: string;
  featuredMetricEn?: string;
  accentClassName: string;
  enabled: boolean;
}

export const EXPERIMENT_LABS: readonly ExperimentLabDefinition[] = [
  {
    id: "automation",
    name: "Agentify｜万物智能",
    nameEn: "Agentify | Omni Intelligence",
    tagline: "把多个软件系统接成一个可持续对话的 Agent 应用",
    taglineEn: "Turn multiple software systems into one durable conversational agent app",
    summary:
      "导入 API 资料，用对话生成可持续对话、可执行、可长期运行的 Agent App，让多个软件系统像一个智能体一样协同工作。",
    summaryEn:
      "Import API materials and use conversation to build a durable, executable, long-running agent app so multiple software systems can collaborate like one intelligent entity.",
    status: "ready",
    icon: Cable,
    featuredMetric: "首个可用智能应用",
    featuredMetricEn: "First production-ready intelligent app",
    accentClassName:
      "border-[hsl(var(--primary))/0.26] bg-[linear-gradient(180deg,hsl(var(--primary))/0.08_0%,transparent_100%)]",
    enabled: true,
  },
];

export function getExperimentLab(labId: string | null | undefined) {
  return EXPERIMENT_LABS.find((item) => item.id === labId) || null;
}

export function localizeExperimentLab(
  lab: ExperimentLabDefinition,
  language: string | null | undefined,
): ExperimentLabDefinition {
  const isZh = (language || "").toLowerCase().startsWith("zh");
  if (isZh) {
    return lab;
  }
  return {
    ...lab,
    name: lab.nameEn || lab.name,
    tagline: lab.taglineEn || lab.tagline,
    summary: lab.summaryEn || lab.summary,
    featuredMetric: lab.featuredMetricEn || lab.featuredMetric,
  };
}
