import { Cable, type LucideIcon } from "lucide-react";

export type ExperimentLabId = "automation";
export type ExperimentLabStatus = "ready" | "alpha" | "planned";

export interface ExperimentLabDefinition {
  id: ExperimentLabId;
  name: string;
  nameKey?: string;
  tagline: string;
  taglineKey?: string;
  summary: string;
  summaryKey?: string;
  status: ExperimentLabStatus;
  icon: LucideIcon;
  featuredMetric: string;
  featuredMetricKey?: string;
  accentClassName: string;
  enabled: boolean;
}

export const EXPERIMENT_LABS: readonly ExperimentLabDefinition[] = [
  {
    id: "automation",
    name: "Agentify | Omni Intelligence",
    nameKey: "experimentLab.labs.automation.name",
    tagline: "Turn multiple software systems into one durable conversational agent app",
    taglineKey: "experimentLab.labs.automation.tagline",
    summary:
      "Import API materials and use conversation to build a durable, executable, long-running agent app so multiple software systems can collaborate like one intelligent entity.",
    summaryKey: "experimentLab.labs.automation.summary",
    status: "ready",
    icon: Cable,
    featuredMetric: "First production-ready intelligent app",
    featuredMetricKey: "experimentLab.labs.automation.featuredMetric",
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
  translate?: (key: string, fallback: string) => string,
): ExperimentLabDefinition {
  void language;
  if (!translate) return lab;
  return {
    ...lab,
    name: lab.nameKey ? translate(lab.nameKey, lab.name) : lab.name,
    tagline: lab.taglineKey ? translate(lab.taglineKey, lab.tagline) : lab.tagline,
    summary: lab.summaryKey ? translate(lab.summaryKey, lab.summary) : lab.summary,
    featuredMetric: lab.featuredMetricKey
      ? translate(lab.featuredMetricKey, lab.featuredMetric)
      : lab.featuredMetric,
  };
}
