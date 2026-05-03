import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Beaker, Bot, FileCog, FileText, FolderKanban, Plus, RefreshCw, Rocket, Sparkles, Trash2, Wrench, Zap } from "lucide-react";

import { agentApi, type TeamAgent } from "../../../api/agent";
import { automationApi, type AgentAppRuntime, type AutomationArtifact, type AutomationIntegration, type AutomationModule, type AutomationProject, type AutomationRun, type AutomationSchedule, type AutomationTaskDraft, type IntegrationAuthType, type IntegrationSpecKind, type RunMode, type ScheduleMode } from "../../../api/automation";
import { ChatConversation } from "../../chat/ChatConversation";
import type { ChatInputComposeRequest, ChatInputQuickActionGroup } from "../../chat/ChatInput";
import { BottomSheetPanel } from "../../mobile/BottomSheetPanel";
import { Button } from "../../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import { ConfirmDialog } from "../../ui/confirm-dialog";
import { EmptyState } from "../../ui/empty-state";
import { Input } from "../../ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../ui/select";
import { Textarea } from "../../ui/textarea";
import { cn } from "../../../utils";

interface AutomationLabWorkspaceProps {
  teamId: string;
  canManage: boolean;
}

type SurfaceKey = "builder" | "apps" | "ops";
type OpsTabKey = "modules" | "runs" | "plans" | "artifacts";
type PublishAction = "none" | "chat" | "run_once" | "schedule" | "monitor";
type TranslateFn = (key: string, fallback: string, options?: Record<string, unknown>) => string;
const OPS_TABS: Array<{ key: OpsTabKey; labelKey: string; fallback: string }> = [
  { key: "modules", labelKey: "experimentLab.opsTabs.modules", fallback: "Apps" },
  { key: "runs", labelKey: "experimentLab.opsTabs.runs", fallback: "Runs" },
  { key: "plans", labelKey: "experimentLab.opsTabs.plans", fallback: "Plans" },
  { key: "artifacts", labelKey: "experimentLab.opsTabs.artifacts", fallback: "Artifacts" },
];

function pretty(value: string | null | undefined, language: string | null | undefined, notSetLabel: string) {
  if (!value) return notSetLabel;
  try {
    const locale = (language || "").toLowerCase().startsWith("zh") ? "zh-CN" : "en-US";
    return new Date(value).toLocaleString(locale, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  } catch {
    return value;
  }
}

function draftNameFromGoal(goal: string, fallback: string) {
  const line = goal.replace(/\r\n/g, "\n").split("\n").find((item) => item.trim())?.trim();
  if (!line) return fallback;
  return line.length > 28 ? `${line.slice(0, 28)}…` : line;
}

function integrationNameFromContent(
  content: string,
  translate: (key: string, fallback: string) => string,
) {
  const firstLine = content
    .replace(/\r\n/g, "\n")
    .split("\n")
    .find((item) => item.trim())
    ?.trim();
  if (!firstLine) return translate("experimentLab.newApiSource", "New API source");
  return firstLine.length > 32 ? `${firstLine.slice(0, 32)}…` : firstLine;
}

function inferIntegrationSpecKind(content: string): IntegrationSpecKind {
  const normalized = content.toLowerCase();
  if (normalized.includes("openapi:") || normalized.includes("swagger:")) {
    return "openapi";
  }
  if (normalized.includes("\"info\"") && normalized.includes("\"paths\"")) {
    return "json";
  }
  if (normalized.includes("curl ")) {
    return "curl";
  }
  if (normalized.includes("postman")) {
    return "postman";
  }
  if (normalized.includes("http://") || normalized.includes("https://")) {
    return "markdown";
  }
  return "text";
}

function looksLikeApiSource(content: string) {
  const value = content.trim();
  if (!value) return false;
  const normalized = value.toLowerCase();
  if (
    normalized.includes("openapi:") ||
    normalized.includes("swagger:") ||
    normalized.includes("postman") ||
    normalized.includes("curl ") ||
    normalized.includes("paths:") ||
    normalized.includes("components:") ||
    normalized.includes("\"paths\"") ||
    normalized.includes("base url") ||
    normalized.includes("endpoint") ||
    normalized.includes("api 文档") ||
    normalized.includes("接口文档")
  ) {
    return true;
  }
  if (/https?:\/\/\S+/i.test(value)) {
    return true;
  }
  if (/\b(GET|POST|PUT|PATCH|DELETE)\s+\/[\w/-]*/i.test(value)) {
    return true;
  }
  return false;
}

function draftLabel(
  draft: AutomationTaskDraft | null | undefined,
  translate: (key: string, fallback: string) => string,
) {
  if (!draft) return translate("experimentLab.newBuilder", "New Builder");
  if (draft.publish_readiness?.ready) return translate("experimentLab.readyToPublish", "Ready to publish");
  if (draft.status === "probing") return translate("experimentLab.probing", "Probing");
  if (draft.status === "failed") return translate("experimentLab.needsFix", "Needs fixes");
  return translate("common.draft", "Draft");
}

function draftTone(draft?: AutomationTaskDraft | null) {
  if (!draft) return "bg-[hsl(var(--ui-surface-panel-strong))] text-muted-foreground";
  if (draft.publish_readiness?.ready) return "bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]";
  if (draft.status === "probing") return "bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]";
  if (draft.status === "failed") return "bg-[hsl(var(--status-error-bg))] text-[hsl(var(--status-error-text))]";
  return "bg-[hsl(var(--ui-surface-panel-strong))] text-muted-foreground";
}

function verificationLabels(
  summary?: {
    verified_http_actions?: number;
    structured_verified_http_actions?: number;
    shell_fallback_verified_http_actions?: number;
  } | null,
  translate?: TranslateFn,
) {
  if (!summary) return [] as string[];
  const labels: string[] = [];
  if (summary.structured_verified_http_actions) {
    labels.push(translate
      ? translate("experimentLab.structuredVerificationCount", "{{count}} structured verification(s)", {
          count: summary.structured_verified_http_actions,
        })
      : `${summary.structured_verified_http_actions} structured verification(s)`);
  }
  if (summary.shell_fallback_verified_http_actions) {
    labels.push(translate
      ? translate("experimentLab.shellFallbackVerificationCount", "{{count}} shell fallback verification(s)", {
          count: summary.shell_fallback_verified_http_actions,
        })
      : `${summary.shell_fallback_verified_http_actions} shell fallback verification(s)`);
  }
  if (!labels.length && summary.verified_http_actions) {
    labels.push(translate
      ? translate("experimentLab.verifiedCallCount", "{{count}} verified call(s)", {
          count: summary.verified_http_actions,
        })
      : `${summary.verified_http_actions} verified call(s)`);
  }
  return labels;
}

function runStatusLabel(status: string | null | undefined, translate: TranslateFn) {
  if (!status) return "";
  const normalized = status.toLowerCase();
  const key = `experimentLab.runStatus.${normalized}`;
  const fallback = status.replace(/_/g, " ");
  return translate(key, fallback);
}

function connectionStatusLabel(status: string | null | undefined, translate: TranslateFn) {
  if (!status) return translate("experimentLab.connectionStatus.unknown", "Unknown");
  const normalized = status.toLowerCase();
  const key = `experimentLab.connectionStatus.${normalized}`;
  const fallback = status.replace(/_/g, " ");
  return translate(key, fallback);
}

function readinessMessageLabel(message: string, translate: TranslateFn) {
  const normalized = message.trim().replace(/\s+/g, "").replace(/[。.]$/, "");
  if (normalized === "还没有完成真实API验证") {
    return translate(
      "experimentLab.readiness.realApiVerificationMissing",
      "Real API verification has not been completed yet.",
    );
  }
  if (normalized === "缺少可用的真实HTTP验证证据") {
    return translate(
      "experimentLab.readiness.realHttpEvidenceMissing",
      "No usable real HTTP verification evidence is available.",
    );
  }
  return message;
}

function readinessMessagesLabel(messages: string[] | undefined, translate: TranslateFn) {
  return (messages || [])
    .map((message) => readinessMessageLabel(message, translate))
    .join(translate("experimentLab.listSeparator", "; "));
}

function surfaceLabel(surface: SurfaceKey, translate: TranslateFn) {
  if (surface === "builder") return translate("experimentLab.surface.buildAgent", "Build agent");
  if (surface === "apps") return translate("experimentLab.surface.useAgent", "Use agent");
  return translate("experimentLab.surface.observeRuns", "Observe runs");
}

function summarizePlan(
  draft: AutomationTaskDraft | null | undefined,
  translate: (key: string, fallback: string) => string,
) {
  if (!draft) {
    return translate(
      "experimentLab.planEmpty",
      "After you send the first message, the system creates a builder draft automatically and carries the current project, default agent, and selected sources into it.",
    );
  }
  const plan = draft.candidate_plan as Record<string, unknown>;
  return (
    (typeof plan.recommended_path === "string" && plan.recommended_path) ||
    (typeof plan.summary === "string" && plan.summary) ||
    translate(
      "experimentLab.planFallback",
      "There is no mature candidate plan yet. Let the agent organize the sources and verify the API first, then decide whether to publish.",
    )
  );
}

function excerptText(value?: string | null, maxChars = 200) {
  if (!value) return "";
  return value.length > maxChars ? `${value.slice(0, maxChars)}…` : value;
}

function normalizePlanText(value: string) {
  return value
    .replace(/\r\n/g, "\n")
    .replace(/```[\s\S]*?```/g, (block) => block.replace(/```/g, ""))
    .replace(/#{1,6}\s*/g, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function runModeLabel(
  mode: RunMode,
  translate: (key: string, fallback: string) => string,
) {
  if (mode === "chat") return translate("experimentLab.runMode.chat", "Chat execution");
  if (mode === "run_once") return translate("experimentLab.runMode.runOnce", "Run once");
  if (mode === "schedule") return translate("experimentLab.runMode.schedule", "Scheduled run");
  return translate("experimentLab.runMode.monitor", "Continuous monitoring");
}

function latestCompletedRun(runs: AutomationRun[]) {
  return runs.find((item) => item.status !== "draft") || null;
}

function workflowStepTone(state: "active" | "complete" | "idle") {
  if (state === "active") {
    return "border-[hsl(var(--primary))/0.24] bg-[hsl(var(--primary))/0.08] text-foreground";
  }
  if (state === "complete") {
    return "border-[hsl(var(--status-success-border))] bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]";
  }
  return "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] text-muted-foreground";
}

function scheduleModeLabel(
  mode: ScheduleMode,
  translate: (key: string, fallback: string) => string,
) {
  return mode === "monitor"
    ? translate("experimentLab.scheduleMode.monitor", "Continuous monitoring")
    : translate("experimentLab.scheduleMode.schedule", "Recurring run");
}

export function AutomationLabWorkspace({ teamId, canManage }: AutomationLabWorkspaceProps) {
  const { t, i18n } = useTranslation();
  const [surface, setSurface] = useState<SurfaceKey>("builder");
  const [builderStage, setBuilderStage] = useState<"chat" | "publish" | "app">("chat");
  const [opsTab, setOpsTab] = useState<OpsTabKey>("modules");
  const [projects, setProjects] = useState<AutomationProject[]>([]);
  const [integrations, setIntegrations] = useState<AutomationIntegration[]>([]);
  const [drafts, setDrafts] = useState<AutomationTaskDraft[]>([]);
  const [modules, setModules] = useState<AutomationModule[]>([]);
  const [runs, setRuns] = useState<AutomationRun[]>([]);
  const [artifacts, setArtifacts] = useState<AutomationArtifact[]>([]);
  const [schedules, setSchedules] = useState<AutomationSchedule[]>([]);
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [selectedDraftId, setSelectedDraftId] = useState<string | null>(null);
  const [selectedModuleId, setSelectedModuleId] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const [sourcesOpen, setSourcesOpen] = useState(false);
  const [contextOpen, setContextOpen] = useState(false);
  const [publishConfirmOpen, setPublishConfirmOpen] = useState(false);
  const [runChatSessionId, setRunChatSessionId] = useState<string | null>(null);
  const [runChatAgentId, setRunChatAgentId] = useState<string | null>(null);
  const [runDetailOpen, setRunDetailOpen] = useState(false);
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null);
  const [appRuntime, setAppRuntime] = useState<AgentAppRuntime | null>(null);
  const [expandedRunIds, setExpandedRunIds] = useState<string[]>([]);
  const [expandedArtifactIds, setExpandedArtifactIds] = useState<string[]>([]);

  const [projectName, setProjectName] = useState("");
  const [projectDescription, setProjectDescription] = useState("");
  const [defaultBuilderAgentId, setDefaultBuilderAgentId] = useState("");
  const [selectedIntegrationIds, setSelectedIntegrationIds] = useState<string[]>([]);
  const [builderComposeRequest, setBuilderComposeRequest] = useState<ChatInputComposeRequest | null>(null);
  const [nextSendAsSource, setNextSendAsSource] = useState(false);
  const [builderNotice, setBuilderNotice] = useState<{
    tone: "info" | "success";
    text: string;
  } | null>(null);

  const [integrationName, setIntegrationName] = useState("");
  const [integrationSpecKind, setIntegrationSpecKind] = useState<IntegrationSpecKind>("openapi");
  const [integrationBaseUrl, setIntegrationBaseUrl] = useState("");
  const [integrationAuthType, setIntegrationAuthType] = useState<IntegrationAuthType>("none");
  const [integrationAuthConfig, setIntegrationAuthConfig] = useState("{}");
  const [integrationSpecContent, setIntegrationSpecContent] = useState("");
  const [integrationSourceFileName, setIntegrationSourceFileName] = useState("");
  const [publishModuleName, setPublishModuleName] = useState("");
  const [publishAction, setPublishAction] = useState<PublishAction>("none");
  const [publishSummaryExpanded, setPublishSummaryExpanded] = useState(false);
  const [scheduleCron, setScheduleCron] = useState("0 9 * * *");
  const [monitorInterval, setMonitorInterval] = useState("300");
  const [monitorInstruction, setMonitorInstruction] = useState("");
  const [publishing, setPublishing] = useState(false);
  const [runningModuleAction, setRunningModuleAction] = useState<{
    moduleId: string;
    mode: RunMode;
  } | null>(null);
  const [creatingPlanMode, setCreatingPlanMode] = useState<"schedule" | "monitor" | null>(null);
  const [togglingScheduleId, setTogglingScheduleId] = useState<string | null>(null);
  const [deletingScheduleId, setDeletingScheduleId] = useState<string | null>(null);
  const [deletingModuleId, setDeletingModuleId] = useState<string | null>(null);
  const [deleteModuleTarget, setDeleteModuleTarget] = useState<AutomationModule | null>(null);
  const [deletingProjectId, setDeletingProjectId] = useState<string | null>(null);
  const [deleteProjectTarget, setDeleteProjectTarget] = useState<AutomationProject | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const builderTurnActiveRef = useRef(false);
  const latestDraftIdRef = useRef<string | null>(null);
  const translate = useCallback<TranslateFn>(
    (key, fallback, options) => t(key, fallback, options),
    [t],
  );
  const formatTime = useCallback(
    (value?: string | null) => pretty(value, i18n.language, t("experimentLab.notSet", "Not set")),
    [i18n.language, t],
  );

  const selectedProject = useMemo(() => projects.find((item) => item.project_id === selectedProjectId) || null, [projects, selectedProjectId]);
  const selectedDraft = useMemo(() => drafts.find((item) => item.draft_id === selectedDraftId) || null, [drafts, selectedDraftId]);
  const selectedModule = useMemo(() => modules.find((item) => item.module_id === selectedModuleId) || modules[0] || null, [modules, selectedModuleId]);
  const selectedRun = useMemo(() => runs.find((item) => item.run_id === selectedRunId) || null, [runs, selectedRunId]);
  const selectedRunArtifacts = useMemo(
    () => artifacts.filter((item) => item.run_id === selectedRunId),
    [artifacts, selectedRunId],
  );
  const builderAgentId = selectedDraft?.driver_agent_id || defaultBuilderAgentId || agents[0]?.id || "";
  const builderAgent = agents.find((item) => item.id === builderAgentId) || null;
  const publishSummary = useMemo(
    () => normalizePlanText(summarizePlan(selectedDraft, (key, fallback) => t(key, fallback))),
    [selectedDraft, t],
  );
  const latestRun = useMemo(() => latestCompletedRun(runs), [runs]);
  const latestRunByModule = useMemo(() => {
    const map = new Map<string, AutomationRun>();
    for (const run of runs) {
      if (!map.has(run.module_id)) {
        map.set(run.module_id, run);
      }
    }
    return map;
  }, [runs]);
  const activeBuilderIntegrationCount = selectedDraft
    ? selectedDraft.summary?.integration_count || selectedDraft.integration_ids.length
    : selectedIntegrationIds.length;
  const builderReady = Boolean(selectedDraft?.publish_readiness?.ready);
  const needsSources = activeBuilderIntegrationCount === 0;
  const workflowSteps = useMemo(() => {
    const builderState: "active" | "complete" | "idle" =
      surface === "builder" && builderStage === "chat"
        ? "active"
        : selectedDraft
          ? "complete"
          : "idle";
    const publishState: "active" | "complete" | "idle" =
      surface === "builder" && builderStage === "publish"
        ? "active"
        : selectedModule
          ? "complete"
          : builderReady
            ? "complete"
            : "idle";
    const appState: "active" | "complete" | "idle" =
      surface === "builder" && builderStage === "app"
        ? "active"
        : selectedModule
          ? "complete"
          : "idle";
    const opsState: "active" | "complete" | "idle" =
      surface === "ops"
        ? "active"
        : latestRun || schedules.length > 0 || artifacts.length > 0
          ? "complete"
          : "idle";

    return [
      {
        key: "builder",
        label: t("experimentLab.workflow.build", "Build"),
        hint: selectedDraft
          ? draftLabel(selectedDraft, translate)
          : t("experimentLab.waitingToStart", "Waiting to start"),
        state: builderState,
      },
      {
        key: "publish",
        label: t("experimentLab.workflow.publish", "Publish"),
        hint: selectedModule ? `v${selectedModule.version}` : builderReady ? t("experimentLab.readyToPublish", "Ready to publish") : t("experimentLab.waitingToPublish", "Waiting to publish"),
        state: publishState,
      },
      {
        key: "app",
        label: t("experimentLab.workflow.app", "App"),
        hint: selectedModule ? selectedModule.name : t("experimentLab.notEntered", "Not entered"),
        state: appState,
      },
      {
        key: "ops",
        label: t("experimentLab.workflow.observe", "Observe"),
        hint: latestRun
          ? `${runModeLabel(latestRun.mode, translate)} · ${runStatusLabel(latestRun.status, translate)}`
          : schedules.length > 0
            ? t("experimentLab.scheduleCount", "{{count}} schedules", { count: schedules.length })
            : t("experimentLab.noRecentAction", "No recent activity"),
        state: opsState,
      },
    ] as const;
  }, [artifacts.length, builderReady, builderStage, latestRun, schedules.length, selectedDraft, selectedModule, surface, t, translate]);

  const loadProjects = useCallback(async () => {
    const res = await automationApi.listProjects(teamId);
    setProjects(res.projects);
    if (!selectedProjectId && res.projects[0]) setSelectedProjectId(res.projects[0].project_id);
  }, [selectedProjectId, teamId]);

  const loadWorkspace = useCallback(async (projectId: string) => {
    const [iRes, dRes, mRes, rRes, aRes, sRes] = await Promise.all([
      automationApi.listIntegrations(teamId, projectId),
      automationApi.listAppDrafts(teamId, projectId),
      automationApi.listApps(teamId, projectId),
      automationApi.listRuns(teamId, projectId),
      automationApi.listArtifacts(teamId, projectId),
      automationApi.listSchedules(teamId, projectId),
    ]);
    setIntegrations(iRes.integrations);
    setDrafts(dRes.app_drafts);
    setModules(mRes.apps);
    setRuns(rRes.runs);
    setArtifacts(aRes.artifacts);
    setSchedules(sRes.schedules);
    setSelectedDraftId((current) => current && dRes.app_drafts.some((item) => item.draft_id === current) ? current : dRes.app_drafts[0]?.draft_id || null);
    setSelectedModuleId((current) => current && mRes.apps.some((item) => item.module_id === current) ? current : mRes.apps[0]?.module_id || null);
  }, [teamId]);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      await loadProjects();
      if (selectedProjectId) await loadWorkspace(selectedProjectId);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("common.loadFailed", "Load failed"));
    } finally {
      setLoading(false);
    }
  }, [loadProjects, loadWorkspace, selectedProjectId]);

  useEffect(() => { void refresh(); }, [refresh]);

  useEffect(() => {
    agentApi.listAgents(teamId, 1, 100).then((res) => {
      setAgents(res.items || []);
      if (!defaultBuilderAgentId && res.items[0]) setDefaultBuilderAgentId(res.items[0].id);
    }).catch((err) => console.error(err));
  }, [defaultBuilderAgentId, teamId]);

  useEffect(() => {
    if (selectedProjectId) void loadWorkspace(selectedProjectId);
  }, [loadWorkspace, selectedProjectId]);

  useEffect(() => {
    if (!selectedModuleId) {
      setAppRuntime(null);
      return;
    }
    let cancelled = false;
    automationApi
      .getAppRuntime(teamId, selectedModuleId)
      .then((runtime) => {
        if (!cancelled) {
          setAppRuntime(runtime);
        }
      })
      .catch((err) => {
        console.error("Failed to load app runtime:", err);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedModuleId, teamId]);

  useEffect(() => {
    const ids = integrations.map((item) => item.integration_id);
    setSelectedIntegrationIds((current) => current.length > 0 ? current.filter((id) => ids.includes(id)) : ids);
  }, [integrations]);

  useEffect(() => {
    if (selectedDraft) setPublishModuleName(selectedDraft.name);
  }, [selectedDraft?.draft_id, selectedDraft?.name]);

  useEffect(() => {
    setPublishSummaryExpanded(false);
  }, [selectedDraft?.draft_id]);

  useEffect(() => {
    latestDraftIdRef.current = selectedDraft?.draft_id || null;
  }, [selectedDraft?.draft_id]);

  useEffect(() => {
    if (!builderNotice) return;
    const timer = window.setTimeout(() => setBuilderNotice(null), 4800);
    return () => window.clearTimeout(timer);
  }, [builderNotice]);

  useEffect(() => {
    if (!selectedProjectId || selectedDraft?.status !== "probing") return;
    const timer = window.setInterval(() => { void loadWorkspace(selectedProjectId); }, 4000);
    return () => window.clearInterval(timer);
  }, [loadWorkspace, selectedDraft?.status, selectedProjectId]);

  useEffect(() => {
    if (!selectedProjectId || selectedRun?.status !== "running") return;
    const timer = window.setInterval(() => {
      void loadWorkspace(selectedProjectId);
    }, 4000);
    return () => window.clearInterval(timer);
  }, [loadWorkspace, selectedProjectId, selectedRun?.run_id, selectedRun?.status]);

  const handleCreateProject = async () => {
    if (!canManage || !projectName.trim()) return;
    const res = await automationApi.createProject({ team_id: teamId, name: projectName, description: projectDescription || null });
    setProjectName("");
    setProjectDescription("");
    await loadProjects();
    setSelectedProjectId(res.project.project_id);
    setWorkspaceOpen(false);
  };

  const handleCreateIntegration = async () => {
    if (!canManage || !selectedProjectId || !integrationSpecContent.trim()) return;
    const normalizedName =
      integrationName.trim() ||
      integrationNameFromContent(integrationSpecContent, (key, fallback) => t(key, fallback));
    const res = await automationApi.createIntegration(selectedProjectId, {
      team_id: teamId,
      project_id: selectedProjectId,
      name: normalizedName,
      spec_kind: integrationSpecKind,
      spec_content: integrationSpecContent,
      source_file_name: integrationSourceFileName || null,
      base_url: integrationBaseUrl || null,
      auth_type: integrationAuthType,
      auth_config: integrationAuthConfig.trim() ? JSON.parse(integrationAuthConfig) : {},
    });
    setIntegrationName("");
    setIntegrationSpecContent("");
    setIntegrationSourceFileName("");
    setIntegrationBaseUrl("");
    setIntegrationAuthConfig("{}");
    setSelectedIntegrationIds((current) => Array.from(new Set([...current, res.integration.integration_id])));
    if (selectedDraft) {
      const updated = await automationApi.updateAppDraft(teamId, selectedDraft.draft_id, { integration_ids: Array.from(new Set([...selectedDraft.integration_ids, res.integration.integration_id])) });
      setDrafts((current) => current.map((item) => item.draft_id === updated.app_draft.draft_id ? updated.app_draft : item));
      setSelectedDraftId(updated.app_draft.draft_id);
    }
    await loadWorkspace(selectedProjectId);
    setBuilderComposeRequest({
      id: `integration:${Date.now()}`,
      text: t(
        "experimentLab.newSourceImportedPrompt",
        "I just imported a new API source named \"{{name}}\". Read it directly from the current workspace, summarize confirmed capabilities, missing information, and recommended next steps. Return a high-level summary and do not repeat large source blocks verbatim.",
        { name: res.integration.name },
      ),
    });
    setSourcesOpen(false);
  };

  const handleBeforeBuilderSend = useCallback(async (content: string, _sessionId: string | null) => {
    const treatAsSource = nextSendAsSource || looksLikeApiSource(content);
    if (!treatAsSource || !selectedProjectId || !content.trim()) {
      return content;
    }

    if (nextSendAsSource) {
      setNextSendAsSource(false);
    }
    const res = await automationApi.createIntegration(selectedProjectId, {
      team_id: teamId,
      project_id: selectedProjectId,
      name: integrationNameFromContent(content, (key, fallback) => t(key, fallback)),
      spec_kind: inferIntegrationSpecKind(content),
      spec_content: content,
      auth_type: "none",
      auth_config: {},
    });

    const nextIntegrationIds = Array.from(
      new Set([...selectedIntegrationIds, res.integration.integration_id]),
    );
    setSelectedIntegrationIds(nextIntegrationIds);
    await loadWorkspace(selectedProjectId);

    const targetDraftId = selectedDraft?.draft_id || latestDraftIdRef.current;
    if (targetDraftId) {
      const updated = await automationApi.updateAppDraft(teamId, targetDraftId, {
        name: selectedDraft?.name || t("experimentLab.newBuilder", "New Builder"),
        goal:
          selectedDraft?.goal ||
          t(
            "experimentLab.defaultBuilderGoal",
            "Build a long-lived, publishable, conversational intelligent app from the imported API sources.",
          ),
        integration_ids: Array.from(
          new Set([...(selectedDraft?.integration_ids || nextIntegrationIds), res.integration.integration_id]),
        ),
      });
      setDrafts((current) =>
        current.map((item) => (item.draft_id === updated.app_draft.draft_id ? updated.app_draft : item)),
      );
      setSelectedDraftId(updated.app_draft.draft_id);
      latestDraftIdRef.current = updated.app_draft.draft_id;
    } else {
      const created = await automationApi.createAppDraft(selectedProjectId, {
        team_id: teamId,
        project_id: selectedProjectId,
        name: t("experimentLab.newBuilder", "New Builder"),
        driver_agent_id: builderAgentId,
        integration_ids: [res.integration.integration_id],
        goal: t(
          "experimentLab.defaultBuilderGoal",
          "Build a long-lived, publishable, conversational intelligent app from the imported API sources.",
        ),
        constraints: [],
        success_criteria: [],
        risk_preference: "balanced",
        create_builder_session: false,
      });
      setDrafts((current) => [created.app_draft, ...current]);
      setSelectedDraftId(created.app_draft.draft_id);
      latestDraftIdRef.current = created.app_draft.draft_id;
    }

    setBuilderNotice({
      tone: "info",
      text: nextSendAsSource
        ? t(
            "experimentLab.archivedSourceModeNotice",
            "Archived \"{{name}}\" in source mode. The agent will treat this message as API source material first.",
            { name: res.integration.name },
          )
        : t(
            "experimentLab.archivedAutoSourceNotice",
            "This message looked like API source material and was archived as \"{{name}}\".",
            { name: res.integration.name },
          ),
    });

    return t(
      "experimentLab.chatSourceImportedPrompt",
      "I just imported a new API source named \"{{name}}\" through chat. Treat this input as source material rather than the final task goal. Read it directly from the current workspace, summarize confirmed capabilities, missing information, verification paths, and risk boundaries. If the real task goal is still missing, ask follow-up questions proactively.",
      { name: res.integration.name },
    );
  }, [builderAgentId, loadWorkspace, nextSendAsSource, selectedDraft, selectedIntegrationIds, selectedProjectId, t, teamId]);

  const handleCreateBuilderSession = useCallback(async (initialMessage: string) => {
    if (!selectedProjectId) throw new Error(t("experimentLab.createProjectFirst", "Create a project first"));
    if (!builderAgentId) throw new Error(t("experimentLab.selectDriverAgentFirst", "Select the driver agent first"));
    const currentDraftId = selectedDraft?.draft_id || latestDraftIdRef.current;
    const currentDraft =
      (currentDraftId && drafts.find((item) => item.draft_id === currentDraftId)) || selectedDraft;
    if (currentDraft?.builder_session_id) return currentDraft.builder_session_id;
    if (currentDraft) {
      const ensured = await automationApi.ensureAppDraftBuilderSession(teamId, currentDraft.draft_id);
      setDrafts((current) => current.map((item) => item.draft_id === ensured.app_draft.draft_id ? ensured.app_draft : item));
      setSelectedDraftId(ensured.app_draft.draft_id);
      latestDraftIdRef.current = ensured.app_draft.draft_id;
      return ensured.builder_session_id;
    }
    const created = await automationApi.createAppDraft(selectedProjectId, {
      team_id: teamId,
      project_id: selectedProjectId,
      name: nextSendAsSource ? t("experimentLab.newBuilder", "New Builder") : draftNameFromGoal(initialMessage, t("experimentLab.newBuilder", "New Builder")),
      driver_agent_id: builderAgentId,
      integration_ids: selectedIntegrationIds,
      goal: nextSendAsSource
        ? t(
            "experimentLab.defaultBuilderGoal",
            "Build a long-lived, publishable, conversational intelligent app from the imported API sources.",
          )
        : initialMessage,
      constraints: [],
      success_criteria: [],
      risk_preference: "balanced",
      create_builder_session: true,
    });
      setDrafts((current) => [created.app_draft, ...current]);
      setSelectedDraftId(created.app_draft.draft_id);
      latestDraftIdRef.current = created.app_draft.draft_id;
      return created.builder_session_id || created.app_draft.builder_session_id || (await automationApi.ensureAppDraftBuilderSession(teamId, created.app_draft.draft_id)).builder_session_id;
  }, [builderAgentId, drafts, nextSendAsSource, selectedDraft, selectedIntegrationIds, selectedProjectId, t, teamId]);

  const handleProbeDraft = async () => {
    if (!selectedDraft) return;
    const res = await automationApi.probeAppDraft(teamId, selectedDraft.draft_id);
    setDrafts((current) => current.map((item) => item.draft_id === res.app_draft.draft_id ? res.app_draft : item));
    setSelectedDraftId(res.app_draft.draft_id);
    setSurface("builder");
    setBuilderStage("chat");
  };

  const openRunDetail = useCallback((runId: string) => {
    setSelectedRunId(runId);
    setRunDetailOpen(true);
  }, []);

  const openRunConversation = useCallback((run: AutomationRun) => {
    const module = modules.find((item) => item.module_id === run.module_id);
    setRunChatSessionId(run.session_id || null);
    setRunChatAgentId(module?.driver_agent_id || null);
  }, [modules]);

  const handlePublish = async () => {
    if (!canManage || !selectedDraft || publishing) return;
    setPublishing(true);
    setError("");
    try {
      const appRes = await automationApi.publishApp(
        teamId,
        selectedDraft.draft_id,
        publishModuleName || selectedDraft.name,
      );
      const publishedApp = appRes.app;
      setSelectedModuleId(publishedApp.module_id);

      if (publishAction === "run_once") {
        const runRes = await automationApi.startAppRun(teamId, publishedApp.module_id, "run_once");
        setRuns((current) => [runRes.run, ...current]);
        openRunDetail(runRes.run.run_id);
        setSurface("ops");
        setOpsTab("runs");
      } else if (publishAction === "schedule" || publishAction === "monitor") {
        const scheduleRes = await automationApi.createAppSchedule(teamId, publishedApp.module_id, {
          mode: publishAction === "monitor" ? "monitor" : "schedule",
          cron_expression: publishAction === "schedule" ? scheduleCron : null,
          poll_interval_seconds: publishAction === "monitor" ? Number(monitorInterval || "300") : null,
          monitor_instruction: publishAction === "monitor" ? monitorInstruction || null : null,
        });
        setSchedules((current) => [scheduleRes.schedule, ...current]);
        setSurface("ops");
        setOpsTab("plans");
      } else if (publishAction === "chat") {
        setSurface("builder");
        setBuilderStage("app");
      } else {
        setSurface("ops");
        setOpsTab("modules");
      }

      if (selectedProjectId) await loadWorkspace(selectedProjectId);
      if (publishAction !== "chat") {
        setBuilderStage("chat");
      }
      setPublishConfirmOpen(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("experimentLab.publishFailed", "Publish failed"));
    } finally {
      setPublishing(false);
    }
  };

  const handleRunModule = async (module: AutomationModule, mode: RunMode) => {
    if (mode === "chat") {
      setSelectedModuleId(module.module_id);
      setSurface("builder");
      setBuilderStage("app");
      return;
    }
    if (runningModuleAction) return;
    setRunningModuleAction({ moduleId: module.module_id, mode });
    setError("");
    try {
      const res = await automationApi.startAppRun(teamId, module.module_id, mode);
      setRuns((current) => [res.run, ...current]);
      setSelectedModuleId(module.module_id);
      setSurface("ops");
      setOpsTab("runs");
      openRunDetail(res.run.run_id);
      if (selectedProjectId) {
        await loadWorkspace(selectedProjectId);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : t("experimentLab.runFailed", "Run failed"));
    } finally {
      setRunningModuleAction(null);
    }
  };

  const handleCreateSchedule = async (mode: "schedule" | "monitor") => {
    if (!canManage || !selectedModule || creatingPlanMode) return;
    setCreatingPlanMode(mode);
    setError("");
    try {
      const res = await automationApi.createAppSchedule(teamId, selectedModule.module_id, {
        mode,
        cron_expression: mode === "schedule" ? scheduleCron : null,
        poll_interval_seconds: mode === "monitor" ? Number(monitorInterval || "300") : null,
        monitor_instruction: mode === "monitor" ? monitorInstruction || null : null,
      });
      setSchedules((current) => [res.schedule, ...current]);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("experimentLab.createPlanFailed", "Failed to create the plan"));
    } finally {
      setCreatingPlanMode(null);
    }
  };

  const handleToggleSchedule = async (schedule: AutomationSchedule) => {
    if (!canManage || togglingScheduleId) return;
    setTogglingScheduleId(schedule.schedule_id);
    setError("");
    try {
      const res = await automationApi.updateSchedule(teamId, schedule.schedule_id, {
        status: schedule.status === "active" ? "paused" : "active",
      });
      setSchedules((current) => current.map((item) => item.schedule_id === schedule.schedule_id ? res.schedule : item));
    } catch (err) {
      setError(err instanceof Error ? err.message : t("experimentLab.updatePlanFailed", "Failed to update the plan"));
    } finally {
      setTogglingScheduleId(null);
    }
  };

  const handleDeleteSchedule = async (scheduleId: string) => {
    if (!canManage || deletingScheduleId) return;
    setDeletingScheduleId(scheduleId);
    setError("");
    try {
      await automationApi.deleteSchedule(teamId, scheduleId);
      setSchedules((current) => current.filter((item) => item.schedule_id !== scheduleId));
    } catch (err) {
      setError(err instanceof Error ? err.message : t("experimentLab.deletePlanFailed", "Failed to delete the plan"));
    } finally {
      setDeletingScheduleId(null);
    }
  };

  const handleDeleteModule = async () => {
    if (!canManage || !deleteModuleTarget || deletingModuleId) return;
    setDeletingModuleId(deleteModuleTarget.module_id);
    setError("");
    try {
      await automationApi.deleteApp(teamId, deleteModuleTarget.module_id);
      setModules((current) =>
        current.filter((item) => item.module_id !== deleteModuleTarget.module_id),
      );
      setRuns((current) =>
        current.filter((item) => item.module_id !== deleteModuleTarget.module_id),
      );
      setSchedules((current) =>
        current.filter((item) => item.module_id !== deleteModuleTarget.module_id),
      );
      if (selectedRun?.module_id === deleteModuleTarget.module_id) {
        setSelectedRunId(null);
        setRunDetailOpen(false);
      }
      if (selectedModuleId === deleteModuleTarget.module_id) {
        const nextModule = modules.find(
          (item) => item.module_id !== deleteModuleTarget.module_id,
        );
        setSelectedModuleId(nextModule?.module_id || null);
        setAppRuntime(null);
      }
      if (selectedProjectId) {
        await loadWorkspace(selectedProjectId);
      }
      setDeleteModuleTarget(null);
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t("experimentLab.deleteAppFailed", "删除应用失败"),
      );
    } finally {
      setDeletingModuleId(null);
    }
  };

  const handleDeleteProject = async () => {
    if (!canManage || !deleteProjectTarget || deletingProjectId) return;
    setDeletingProjectId(deleteProjectTarget.project_id);
    setError("");
    try {
      await automationApi.deleteProject(teamId, deleteProjectTarget.project_id);
      const remainingProjects = projects.filter(
        (item) => item.project_id !== deleteProjectTarget.project_id,
      );
      setProjects(remainingProjects);
      if (selectedProjectId === deleteProjectTarget.project_id) {
        const nextProject = remainingProjects[0] || null;
        setSelectedProjectId(nextProject?.project_id || null);
        setSelectedDraftId(null);
        setSelectedModuleId(null);
        setSelectedRunId(null);
        setRunDetailOpen(false);
        setAppRuntime(null);
        setIntegrations([]);
        setDrafts([]);
        setModules([]);
        setRuns([]);
        setArtifacts([]);
        setSchedules([]);
        if (nextProject) {
          await loadWorkspace(nextProject.project_id);
        }
      }
      setDeleteProjectTarget(null);
      setWorkspaceOpen(false);
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t("experimentLab.deleteProjectFailed", "删除项目失败"),
      );
    } finally {
      setDeletingProjectId(null);
    }
  };

  const builderQuickActions = useMemo<ChatInputQuickActionGroup[]>(() => [
    {
      key: "discovery",
      label: t("experimentLab.quickActions.discovery", "Discovery"),
      actions: [
        { key: "discover", label: t("experimentLab.quickActions.reviewSources", "Review current sources"), description: t("experimentLab.quickActions.reviewSourcesDescription", "Ask the agent to summarize imported API capabilities, missing information, and recommended next steps."), onSelect: () => setBuilderComposeRequest({ id: `compose:${Date.now()}:discover`, text: t("experimentLab.quickActions.reviewSourcesPrompt", "Please review the currently imported API sources first, summarize the most relevant software and API capabilities, identify missing information, and recommend the next steps. Return a high-level summary by default.") }) },
        { key: "plan", label: t("experimentLab.quickActions.smallestPlan", "Generate the smallest viable plan"), description: t("experimentLab.quickActions.smallestPlanDescription", "Generate the smallest verifiable and runnable path around the current goal."), onSelect: () => setBuilderComposeRequest({ id: `compose:${Date.now()}:plan`, text: t("experimentLab.quickActions.smallestPlanPrompt", "Please propose the smallest viable execution plan for the current goal, including the software involved, key actions, verification method, risks, and the recommended run mode.") }) },
      ],
    },
    {
      key: "verify",
      label: t("experimentLab.quickActions.verification", "Verification & execution"),
      actions: [
        { key: "probe", label: t("experimentLab.quickActions.safeProbe", "Run safe probe verification only"), description: t("experimentLab.quickActions.safeProbeDescription", "Prefer discover / probe / verify and avoid risky real write operations."), onSelect: () => setBuilderComposeRequest({ id: `compose:${Date.now()}:probe`, text: t("experimentLab.quickActions.safeProbePrompt", "Please run safe probe verification first. Check the API path, parameters, and auth method without performing risky real write operations.") }) },
        { key: "execute", label: t("experimentLab.quickActions.executeOneCall", "Execute one real call"), description: t("experimentLab.quickActions.executeOneCallDescription", "Real execution is allowed, but risky write operations must be confirmed first."), onSelect: () => setBuilderComposeRequest({ id: `compose:${Date.now()}:execute`, text: t("experimentLab.quickActions.executeOneCallPrompt", "Please execute one real call to verify the current plan. If risky write operations are involved, ask for explicit confirmation before continuing.") }) },
      ],
    },
  ], [t]);

  const handleBuilderProcessingChange = useCallback((processing: boolean) => {
    if (processing) {
      builderTurnActiveRef.current = true;
      return;
    }
    if (!builderTurnActiveRef.current || !selectedDraft?.draft_id) {
      return;
    }
    builderTurnActiveRef.current = false;
    void automationApi
      .syncAppDraftFromBuilder(teamId, selectedDraft.draft_id)
      .then((res) => {
        setDrafts((current) =>
          current.map((item) => (item.draft_id === res.app_draft.draft_id ? res.app_draft : item)),
        );
        setSelectedDraftId(res.app_draft.draft_id);
        if (res.sync_state === "updated") {
          setBuilderNotice({
            tone: res.app_draft.publish_readiness?.ready ? "success" : "info",
            text: res.app_draft.publish_readiness?.ready
              ? t(
                  "experimentLab.builderSyncedReady",
                  "Builder results synced. The current Agent App draft can now be published directly.",
                )
              : t(
                  "experimentLab.builderSynced",
                  "Builder results synced into the current Agent App draft.",
                ),
          });
        }
      })
      .catch((err) => {
        console.error("Failed to sync builder draft:", err);
      });
  }, [selectedDraft?.draft_id, t, teamId]);

  const toggleExpandedRun = useCallback((runId: string) => {
    setExpandedRunIds((current) =>
      current.includes(runId) ? current.filter((item) => item !== runId) : [...current, runId],
    );
  }, []);

  const toggleExpandedArtifact = useCallback((artifactId: string) => {
    setExpandedArtifactIds((current) =>
      current.includes(artifactId)
        ? current.filter((item) => item !== artifactId)
        : [...current, artifactId],
    );
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col gap-4">
      <div className="rounded-[24px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] px-4 py-4 sm:px-5">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
          <div className="min-w-0">
            <div className="inline-flex items-center gap-2 rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.92] px-3 py-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground"><Beaker className="h-3.5 w-3.5" />{t("experimentLab.heroBadge", "Agentify | Universal Intelligence")}</div>
            <h2 className="mt-2 text-xl font-semibold tracking-tight text-foreground sm:text-2xl">{t("experimentLab.heroTitle", "把常用软件能力变成可持续运行的智能应用")}</h2>
            <p className="mt-1 max-w-3xl text-sm leading-6 text-muted-foreground">{t("experimentLab.heroDescription", "你只需要提供接口资料和业务目标，智能体会自己理解 API、验证可行性、整理方案，并把它发布成可以反复使用的智能流程。")}</p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" className="rounded-full" onClick={() => setWorkspaceOpen(true)}><FolderKanban className="mr-1 h-4 w-4" />{selectedProject?.name || t("experimentLab.selectProject", "选择项目")}</Button>
            <Button variant="outline" className="rounded-full" onClick={() => setSourcesOpen(true)}><FileCog className="mr-1 h-4 w-4" />{t("experimentLab.sourcesAndConnections", "资料与连接")}</Button>
            <Button variant="outline" className="rounded-full" onClick={() => void refresh()} disabled={loading}><RefreshCw className={cn("mr-1 h-4 w-4", loading && "animate-spin")} />{t("common.reload", "重新加载")}</Button>
          </div>
        </div>
        <div className="mt-4 flex flex-wrap gap-2">
          {(["builder", "apps", "ops"] as SurfaceKey[]).map((item) => {
            const active = item === "ops" ? surface === "ops" : surface === "builder" && (item === "builder" ? builderStage !== "app" : builderStage === "app");
            return (
            <button key={item} type="button" onClick={() => { if (item === "ops") { setSurface("ops"); } else { setSurface("builder"); setBuilderStage(item === "apps" ? "app" : "chat"); } }} className={cn("inline-flex items-center gap-2 rounded-full border px-3.5 py-2 text-sm font-medium transition-colors", active ? "border-[hsl(var(--primary))/0.2] bg-[hsl(var(--primary))] text-[hsl(var(--primary-foreground))]" : "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] text-foreground hover:bg-muted/35")}>
              {surfaceLabel(item, translate)}
            </button>
          );})}
        </div>
      </div>

      {(publishing || runningModuleAction || creatingPlanMode || latestRun || builderNotice) ? (
        <Card className="ui-section-panel">
          <CardContent className="flex flex-col gap-3 p-4 md:flex-row md:items-center md:justify-between">
            <div className="min-w-0">
                <div className="text-sm font-semibold text-foreground">{t("experimentLab.latestActivity", "Latest activity")}</div>
              <div className="mt-1 text-xs leading-5 text-muted-foreground">
                {publishing
                  ? t("experimentLab.activityPublishing", "Publishing a new Agent app version.")
                  : runningModuleAction
                    ? t("experimentLab.activityRunning", "Triggering a new run.")
                    : creatingPlanMode
                      ? t("experimentLab.activityCreatingPlan", "Creating a plan.")
                      : builderNotice
                        ? builderNotice.text
                        : latestRun
                          ? `${t("experimentLab.latestRunPrefix", "Latest run: ")}${runModeLabel(latestRun.mode, translate)} · ${runStatusLabel(latestRun.status, translate)} · ${formatTime(latestRun.created_at)}`
                          : t("experimentLab.noRecentAction", "No recent activity")}
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              {publishing ? (
                <Button variant="outline" size="sm" disabled>
                  {t("experimentLab.publishing", "Publishing…")}
                </Button>
              ) : null}
              {runningModuleAction ? (
                <Button variant="outline" size="sm" disabled>
                  {t("experimentLab.running", "Running…")}
                </Button>
              ) : null}
              {!publishing && !runningModuleAction && latestRun ? (
                <Button variant="outline" size="sm" onClick={() => openRunDetail(latestRun.run_id)}>
                  {t("experimentLab.viewLatestRun", "View latest run")}
                </Button>
              ) : null}
            </div>
          </CardContent>
        </Card>
      ) : null}

      {selectedProject ? (
        <Card className="ui-section-panel">
          <CardContent className="flex flex-col gap-4 p-4">
            <div className="flex flex-col gap-2 md:flex-row md:items-center md:justify-between">
              <div>
                <div className="text-sm font-semibold text-foreground">{t("experimentLab.currentWorkflow", "Current workflow")}</div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {t("experimentLab.currentWorkflowDescription", "This view connects build, publish, use, and observe into one line. After finishing one step, move directly to the next or inspect the result.")}
                </div>
              </div>
              <div className="flex flex-wrap gap-2">
                {selectedDraft && !builderReady ? (
                  <Button variant="outline" size="sm" onClick={() => { setSurface("builder"); setBuilderStage("chat"); }}>
                    {t("experimentLab.continueBuilding", "Continue building")}
                  </Button>
                ) : null}
                {builderReady ? (
                  <Button variant="outline" size="sm" onClick={() => { setSurface("builder"); setBuilderStage("publish"); }}>
                    {t("experimentLab.goPublish", "Go publish")}
                  </Button>
                ) : null}
                {selectedModule ? (
                  <Button variant="outline" size="sm" onClick={() => { setSurface("builder"); setBuilderStage("app"); }}>
                    {t("experimentLab.openApp", "Open app")}
                  </Button>
                ) : null}
                {latestRun ? (
                  <Button variant="outline" size="sm" onClick={() => openRunDetail(latestRun.run_id)}>
                    {t("experimentLab.viewResult", "View result")}
                  </Button>
                ) : null}
              </div>
            </div>

            <div className="grid gap-3 md:grid-cols-4">
              {workflowSteps.map((step) => (
                <button
                  key={step.key}
                  type="button"
                  onClick={() => {
                    if (step.key === "builder") {
                      setSurface("builder");
                      setBuilderStage("chat");
                    } else if (step.key === "publish") {
                      setSurface("builder");
                      setBuilderStage("publish");
                    } else if (step.key === "app") {
                      setSurface("builder");
                      setBuilderStage("app");
                    } else {
                      setSurface("ops");
                    }
                  }}
                  className={cn(
                    "rounded-[18px] border px-4 py-3 text-left transition-colors",
                    workflowStepTone(step.state),
                  )}
                >
                  <div className="text-[11px] font-semibold uppercase tracking-[0.08em]">
                    {step.label}
                  </div>
                  <div className="mt-2 text-sm font-medium">{step.hint}</div>
                </button>
              ))}
            </div>
          </CardContent>
        </Card>
      ) : null}

      {(() => {
        if (!selectedProject) {
          return (
        <Card className="ui-section-panel min-h-[520px]"><CardContent className="flex h-full flex-col items-center justify-center px-6 py-10 text-center"><div className="inline-flex h-16 w-16 items-center justify-center rounded-[22px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.94]"><Bot className="h-7 w-7 text-foreground" /></div><h3 className="mt-6 text-2xl font-semibold tracking-tight">{t("experimentLab.emptyProjectTitle", "先创建一个项目，再开始你的智能应用")}</h3><p className="mt-3 max-w-xl text-sm leading-6 text-muted-foreground">{t("experimentLab.emptyProjectDescription", "项目只是这次智能应用的容器。创建之后，你就可以直接通过对话导入接口资料、描述目标，并让智能体开始构建。")}</p><Button className="mt-6 rounded-full" onClick={() => setWorkspaceOpen(true)} disabled={!canManage}><Plus className="mr-1 h-4 w-4" />{t("experimentLab.createProject", "创建项目")}</Button></CardContent></Card>
          );
        }
        if (surface !== "ops" && builderStage === "chat") {
          return (
        <div className="grid gap-4">
          <Card className="ui-section-panel min-h-[calc(100vh-260px)] overflow-hidden">
            <CardContent className="flex h-full min-h-[calc(100vh-270px)] flex-col p-0">
              <div className="border-b border-[hsl(var(--ui-line-soft))/0.72] px-4 py-3 sm:px-5">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold text-foreground">{selectedDraft?.name || t("experimentLab.newBuilder", "新的智能构建")}</span>
                  <span className={cn("inline-flex items-center rounded-full px-2.5 py-1 text-[11px] font-semibold", draftTone(selectedDraft))}>{draftLabel(selectedDraft, translate)}</span>
                  <span className="inline-flex items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.9] px-2.5 py-1 text-[11px] text-muted-foreground">
                    {needsSources
                      ? t("experimentLab.waitingForSources", "等待导入资料")
                      : t("experimentLab.sourcesConnected", "{{count}} 个资料已接入", {
                          count: activeBuilderIntegrationCount,
                        })}
                  </span>
                  <span className="inline-flex items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.9] px-2.5 py-1 text-[11px] text-muted-foreground">
                    {builderAgent?.name || t("experimentLab.noAgentSelected", "未选择 Agent")}
                  </span>
                  <div className="ml-auto flex flex-wrap gap-2">
                    <Button variant="outline" size="sm" className="rounded-full" onClick={() => setSelectedDraftId(null)}>{t("experimentLab.newBuilder", "新 Builder")}</Button>
                    <Button variant="outline" size="sm" className="rounded-full" onClick={() => setContextOpen(true)}>{t("common.draft", "草稿")}</Button>
                    <Button size="sm" className="rounded-full" onClick={selectedDraft?.publish_readiness?.ready ? () => { setBuilderStage("publish"); setSurface("builder"); } : () => void handleProbeDraft()} disabled={!selectedDraft}>{selectedDraft?.publish_readiness?.ready ? t("experimentLab.enterPublish", "进入发布") : t("experimentLab.checkPublish", "检查发布")}</Button>
                  </div>
                </div>
                <div className="mt-2 text-sm text-muted-foreground">{selectedDraft ? summarizePlan(selectedDraft, translate) : t("experimentLab.firstMessageBootstrapsBuilder", "After the first message, the system starts a new intelligent build and carries in the current project, default agent, and selected sources.")}</div>
                {selectedDraft?.publish_readiness?.issues?.length ? (
                  <div className="mt-3 rounded-[16px] border border-[hsl(var(--status-error-border))] bg-[hsl(var(--status-error-bg))] px-3 py-2 text-xs leading-5 text-[hsl(var(--status-error-text))]">
                    {t("experimentLab.publishMissing", "发布前还缺：")}
                    {" "}
                    {readinessMessagesLabel(selectedDraft.publish_readiness.issues, translate)}
                  </div>
                ) : null}
                {selectedDraft?.publish_readiness?.warnings?.length ? (
                  <div className="mt-2 rounded-[16px] border border-[hsl(var(--status-info-border))] bg-[hsl(var(--status-info-bg))] px-3 py-2 text-xs leading-5 text-[hsl(var(--status-info-text))]">
                    {t("common.note", "注意：")}
                    {" "}
                    {readinessMessagesLabel(selectedDraft.publish_readiness.warnings, translate)}
                  </div>
                ) : null}
              </div>
              <div className="min-h-0 flex-1">
                {builderNotice ? (
                  <div
                    className={cn(
                      "border-b px-4 py-2.5 text-xs leading-5 sm:px-5",
                      builderNotice.tone === "success"
                        ? "border-[hsl(var(--status-success-border))] bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]"
                        : "border-[hsl(var(--status-info-border))] bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]",
                    )}
                  >
                    {builderNotice.text}
                  </div>
                ) : null}
                <div className="border-b border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.72] px-4 py-3 sm:px-5">
                  <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                    <div className="min-w-0">
                      <div className="text-sm font-medium text-foreground">
                        {nextSendAsSource ? t("experimentLab.nextMessageIsSource", "下一条消息会直接当成接口资料处理") : t("experimentLab.autoDetectGoalOrSource", "系统会自动识别你发送的是目标还是资料")}
                      </div>
                      <div className="mt-1 text-xs leading-5 text-muted-foreground">
                        {nextSendAsSource
                          ? t("experimentLab.sourceModeHint", "你可以直接发送 OpenAPI、Postman、curl、接口链接或一段说明。系统会先把它收进资料，再让智能体继续分析。")
                          : t("experimentLab.autoDetectHint", "直接贴接口链接、文档说明、curl 或 OpenAPI 时，系统会优先把它当成资料；只有识别不准时，再手动切换。")}
                      </div>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Button
                        variant={nextSendAsSource ? "default" : "outline"}
                        className="rounded-full"
                        onClick={() => setNextSendAsSource((value) => !value)}
                      >
                        <FileCog className="mr-1 h-4 w-4" />
                        {nextSendAsSource ? t("experimentLab.exitSourceMode", "Exit source mode") : t("experimentLab.switchToSourceMode", "Switch to source mode")}
                      </Button>
                      <Button
                        variant="outline"
                        className="rounded-full"
                        onClick={() =>
                          setBuilderComposeRequest({
                            id: `quick-help:${Date.now()}`,
                            text: t(
                              "experimentLab.guideFirstPrompt",
                              "Before I continue, tell me which information you need first: API sources, connection details, the task goal, validation criteria, or run-mode preferences.",
                            ),
                          })
                        }
                      >
                        {t("experimentLab.letAgentGuideFirst", "Let the agent guide first")}
                      </Button>
                    </div>
                  </div>
                </div>
                {needsSources ? (
                  <div className="border-b border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.55] px-4 py-3 sm:px-5">
                    <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                      <div className="min-w-0">
                        <div className="text-sm font-medium text-foreground">{t("experimentLab.giveApiMaterialFirst", "Give the API material to the agent first")}</div>
                        <div className="mt-1 text-xs leading-5 text-muted-foreground">
                          {t("experimentLab.giveApiMaterialDescription", "You can describe the goal directly, or add OpenAPI, Postman, curl, endpoint links, or documentation first. The system only needs minimal source material, not a full form.")}
                        </div>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        <Button variant="outline" size="sm" className="rounded-full" onClick={() => setSourcesOpen(true)}>
                          <FileCog className="mr-1 h-4 w-4" />
                          {t("experimentLab.manageSources", "Manage sources")}
                        </Button>
                        <Button
                          size="sm"
                          className="rounded-full"
                          onClick={() =>
                    setBuilderComposeRequest({
                              id: `compose:${Date.now()}:ingest`,
                              text: t(
                                "experimentLab.agentGuideMePrompt",
                                "I’m ready to start building a new intelligent app. Tell me the minimal API source material, connection details, and goal description you need, then help me explore, verify, and shape a reusable app from them.",
                              ),
                            })
                          }
                        >
                          <Sparkles className="mr-1 h-4 w-4" />
                          {t("experimentLab.letAgentGuideMe", "Let the agent guide me")}
                        </Button>
                      </div>
                    </div>
                  </div>
                ) : null}
                {builderReady ? (
                  <div className="border-b border-[hsl(var(--status-success-border))] bg-[hsl(var(--status-success-bg))] px-4 py-3 sm:px-5">
                    <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
                      <div className="min-w-0">
                        <div className="text-sm font-medium text-[hsl(var(--status-success-text))]">{t("experimentLab.builderReadyTitle", "This builder run is ready to publish")}</div>
                        <div className="mt-1 text-xs leading-5 text-[hsl(var(--status-success-text))/0.82]">
                          {t("experimentLab.builderReadyDescription", "The agent has already organized a runnable plan. You can go straight to publish and freeze it as a reusable app for the team.")}
                        </div>
                      </div>
                      <Button size="sm" className="rounded-full" onClick={() => { setBuilderStage("publish"); setSurface("builder"); }}>
                        <Rocket className="mr-1 h-4 w-4" />
                        {t("experimentLab.goPublish", "Go publish")}
                      </Button>
                    </div>
                  </div>
                ) : null}
                <ChatConversation sessionId={selectedDraft?.builder_session_id ?? null} agentId={builderAgentId} agentName={builderAgent?.name || t("experimentLab.builderAgentFallback", "Builder Agent")} agent={builderAgent} headerVariant="compact" teamId={teamId} createSession={handleCreateBuilderSession} beforeSend={handleBeforeBuilderSend} composeRequest={builderComposeRequest} inputQuickActionGroups={builderQuickActions} composerActions={<button type="button" onClick={() => setSourcesOpen(true)} className="inline-flex h-9 items-center gap-1 rounded-[12px] border border-border/70 bg-background px-2.5 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/45 sm:h-10 sm:text-[12px]">{t("experimentLab.sources", "Sources")}</button>} composerCollapsedActions={<button type="button" onClick={() => setWorkspaceOpen(true)} className="flex w-full items-center gap-3 rounded-[18px] border border-border/70 bg-card/92 px-4 py-3 text-left transition-colors hover:bg-accent/30"><div className="min-w-0"><div className="text-[13px] font-medium text-foreground">{t("teamNav.workspace", "Workspace")}</div><div className="mt-0.5 text-[11px] leading-4 text-muted-foreground">{t("experimentLab.workspaceHint", "Switch projects, change the default agent, or start a new builder draft.")}</div></div></button>} headerActions={<><button type="button" onClick={() => setSourcesOpen(true)} className="inline-flex h-8 items-center gap-1 rounded-full border border-border/70 px-3 text-[11px] font-medium text-muted-foreground transition-colors hover:border-primary/25 hover:bg-muted/45 hover:text-foreground"><FileCog className="h-3.5 w-3.5" />{t("experimentLab.sources", "Sources")}</button><button type="button" onClick={() => setContextOpen(true)} className="inline-flex h-8 items-center gap-1 rounded-full border border-border/70 px-3 text-[11px] font-medium text-muted-foreground transition-colors hover:border-primary/25 hover:bg-muted/45 hover:text-foreground"><Sparkles className="h-3.5 w-3.5" />{t("common.draft", "Draft")}</button></>} onProcessingChange={handleBuilderProcessingChange} onError={setError} />
              </div>
            </CardContent>
          </Card>
        </div>
          );
        }
        if (surface !== "ops" && builderStage === "app") {
          return (
        <div className="grid gap-4 xl:grid-cols-[320px_minmax(0,1fr)]">
          <Card className="ui-section-panel">
            <CardContent className="space-y-3 p-4">
              <div>
                <div className="text-sm font-semibold text-foreground">{t("experimentLab.publishedAgents", "Published agents")}</div>
                <div className="mt-1 text-xs leading-5 text-muted-foreground">{t("experimentLab.publishedAgentsDescription", "Each published app keeps its own persistent chat entry, and run results are written back here.")}</div>
              </div>
              {modules.length > 0 ? modules.map((module) => (
                <div key={module.module_id} className={cn("rounded-[18px] border px-4 py-4 transition-colors", selectedModuleId === module.module_id ? "border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.08]" : "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88]")}>
                  <button type="button" onClick={() => setSelectedModuleId(module.module_id)} className="w-full text-left">
                    <div className="text-sm font-semibold text-foreground">{module.name}</div>
                    <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
                      <span>v{module.version}</span>
                      <span>{t("experimentLab.systemCount", "{{count}} systems", { count: module.summary?.integration_count || module.integration_ids.length })}</span>
                      {module.summary?.native_execution_ready ? <span>{t("experimentLab.nativeExecution", "Native execution")}</span> : null}
                    </div>
                  </button>
                  {canManage ? (
                    <div className="mt-3 flex justify-end">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setDeleteModuleTarget(module)}
                        disabled={Boolean(deletingModuleId)}
                      >
                        <Trash2 className="mr-1 h-4 w-4" />
                        {t("common.delete", "删除")}
                      </Button>
                    </div>
                  ) : null}
                </div>
              )) : <EmptyState icon={<Bot className="h-5 w-5" />} message={t("experimentLab.noPublishedApps", "还没有已发布 Agent。")} />}
            </CardContent>
          </Card>
          <Card className="ui-section-panel min-h-[calc(100vh-280px)] overflow-hidden">
            <CardContent className="flex h-full min-h-[calc(100vh-290px)] flex-col p-0">
              {!selectedModule ? (
                <div className="flex h-full items-center justify-center p-8">
                  <EmptyState icon={<Bot className="h-5 w-5" />} message={t("experimentLab.publishAppFirst", "先发布一个 Agent 应用，再在这里持续对话。")} />
                </div>
              ) : (
                <>
                  <div className="border-b border-[hsl(var(--ui-line-soft))/0.72] px-4 py-4 sm:px-5">
                    <div className="flex flex-wrap items-center gap-2">
                      <div className="text-lg font-semibold text-foreground">{selectedModule.name}</div>
                      <span className="inline-flex items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.88] px-2.5 py-1 text-[11px] text-muted-foreground">v{selectedModule.version}</span>
                      {selectedModule.summary?.native_execution_ready ? <span className="inline-flex items-center rounded-full border border-[hsl(var(--status-success-border))] bg-[hsl(var(--status-success-bg))] px-2.5 py-1 text-[11px] text-[hsl(var(--status-success-text))]">{t("experimentLab.nativeApiReady", "原生 API 就绪")}</span> : null}
                    </div>
                    <div className="mt-2 text-sm leading-6 text-muted-foreground">
                      {t("experimentLab.appRuntimeDescription", "这个 Agent App 会持续记住已验证的 API 路径、最近运行结果和当前上下文。你可以直接在这里继续下达目标、查询状态或触发动作。")}
                    </div>
                    <div className="mt-3 flex flex-wrap gap-2">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => void handleRunModule(selectedModule, "run_once")}
                        disabled={Boolean(runningModuleAction)}
                      >
                        {runningModuleAction?.moduleId === selectedModule.module_id &&
                        runningModuleAction.mode === "run_once"
                          ? t("experimentLab.running", "运行中…")
                          : t("experimentLab.runMode.runOnce", "运行一次")}
                      </Button>
                      <Button variant="outline" size="sm" onClick={() => { setSurface("ops"); setOpsTab("plans"); }}>{t("experimentLab.plansAndMonitoring", "计划与监控")}</Button>
                      <Button variant="outline" size="sm" onClick={() => { setSurface("ops"); setOpsTab("artifacts"); }}>{t("experimentLab.recentArtifacts", "最近产物")}</Button>
                      {canManage ? (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setDeleteModuleTarget(selectedModule)}
                          disabled={Boolean(deletingModuleId)}
                        >
                          <Trash2 className="mr-1 h-4 w-4" />
                          {t("experimentLab.deleteApp", "删除应用")}
                        </Button>
                      ) : null}
                    </div>
                  </div>
                  <div className="border-b border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.62] px-4 py-3 text-xs text-muted-foreground sm:px-5">
                    {appRuntime
                      ? t("experimentLab.runtimeSummary", "最近运行 {{runs}} 次，活跃计划 {{schedules}} 条，最近产物 {{artifacts}} 个。", {
                          runs: appRuntime.recent_runs.length,
                          schedules: appRuntime.active_schedules.length,
                          artifacts: appRuntime.recent_artifacts.length,
                        })
                      : t("experimentLab.preparingRuntimeContext", "正在准备应用运行时上下文。")}
                  </div>
                  <div className="border-b border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--status-info-bg))/0.55] px-4 py-3 text-xs text-[hsl(var(--status-info-text))] sm:px-5">
                    {t("experimentLab.runtimeInitializedNotice", "应用运行时已初始化。只有你发送消息时才会开始执行；初始化本身不会触发一次空对话。")}
                  </div>
                  <ChatConversation sessionId={appRuntime?.runtime_session_id || selectedModule.runtime_session_id || null} agentId={selectedModule.driver_agent_id} agentName={selectedModule.name} agent={agents.find((item) => item.id === selectedModule.driver_agent_id) || undefined} teamId={teamId} headerVariant="compact" onError={setError} />
                </>
              )}
            </CardContent>
          </Card>
        </div>
          );
        }
        if (surface !== "ops" && builderStage === "publish") {
          return (
        <Card className="ui-section-panel">
          <CardContent className="space-y-6 p-6">
            {!selectedDraft ? (
              <EmptyState icon={<Sparkles className="h-5 w-5" />} message={t("experimentLab.createDraftFirst", "先在 Builder 里创建一个草稿。")} action={<Button variant="outline" onClick={() => setBuilderStage("chat")}>{t("experimentLab.backToBuilder", "回到 Builder")}</Button>} />
            ) : !selectedDraft.publish_readiness?.ready ? (
              <EmptyState icon={<Wrench className="h-5 w-5" />} message={t("experimentLab.draftNotPublishable", "这个草稿还没有达到可发布状态，先回到 Builder 继续验证。")} action={<Button variant="outline" onClick={() => setBuilderStage("chat")}>{t("experimentLab.backToBuilder", "返回 Builder")}</Button>} />
            ) : (
              <>
                <div className="space-y-2">
                  <div className="inline-flex items-center gap-2 rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.92] px-3 py-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                    <Rocket className="h-3.5 w-3.5" />
                    {t("experimentLab.publish", "发布")}
                  </div>
                  <h3 className="text-2xl font-semibold tracking-tight text-foreground">{t("experimentLab.publishHeading", "把这轮智能结果发布成长期可用应用")}</h3>
                  <p className="text-sm leading-6 text-muted-foreground">
                    {t("experimentLab.publishDescription", "智能体已经把接口路径、验证结果和推荐运行方式整理好了。现在你只需要给它一个稳定名字，并决定发布后的第一种运行方式。")}
                  </p>
                </div>

                <div className="rounded-[22px] border border-[hsl(var(--primary))/0.16] bg-[hsl(var(--primary))/0.06] px-5 py-5">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.currentCapabilitySummary", "当前能力摘要")}</div>
                      <div className="mt-1 text-xs leading-5 text-muted-foreground">{t("experimentLab.currentCapabilitySummaryHint", "发布前先快速确认这轮 Builder 沉淀下来的核心能力和运行方式。")}</div>
                    </div>
                    {publishSummary.length > 280 ? (
                      <button
                        type="button"
                        className="shrink-0 text-xs font-medium text-foreground"
                        onClick={() => setPublishSummaryExpanded((value) => !value)}
                      >
                        {publishSummaryExpanded ? t("experimentLab.collapseDetails", "收起详情") : t("experimentLab.expandDetails", "展开详情")}
                      </button>
                    ) : null}
                  </div>
                  <div className="mt-3 text-sm leading-7 text-foreground">
                    {publishSummaryExpanded ? publishSummary : excerptText(publishSummary, 280)}
                  </div>
                </div>

                <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(240px,300px)]">
                  <div className="space-y-4">
                    <div className="space-y-2">
                    <label className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.appName", "应用名称")}</label>
                      <Input value={publishModuleName} onChange={(e) => setPublishModuleName(e.target.value)} placeholder={t("experimentLab.appNamePlaceholder", "为这个智能应用取个名字")} />
                    </div>

                    <div className="flex flex-wrap gap-2">
                      {[
                        selectedProject?.name || t("experimentLab.noProjectSelected", "未选择项目"),
                        t("experimentLab.sourcesConnected", "{{count}} 个资料已接入", { count: selectedDraft.summary?.integration_count || selectedDraft.integration_ids.length }),
                        ...(verificationLabels(selectedDraft.summary, translate).length
                          ? verificationLabels(selectedDraft.summary, translate)
                          : [t("experimentLab.waitingForValidationCalls", "等待验证调用")]),
                      ].map((label) => (
                        <span
                          key={label}
                          className="inline-flex items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] px-3 py-1.5 text-xs text-muted-foreground"
                        >
                          {label}
                        </span>
                      ))}
                    </div>
                  </div>

                <div className="rounded-[22px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.65] px-4 py-4">
                  <div className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.postPublishAction", "发布后先怎么用")}</div>
                    <div className="mt-3 space-y-2">
                      {[["none", t("experimentLab.publishAction.none", "先保存下来")], ["chat", t("experimentLab.publishAction.chat", "马上对话试用")], ["run_once", t("experimentLab.publishAction.runOnce", "先运行一次")], ["schedule", t("experimentLab.publishAction.schedule", "按周期运行")], ["monitor", t("experimentLab.publishAction.monitor", "持续观察并运行")]].map(([value, label]) => (
                        <button key={value} type="button" onClick={() => setPublishAction(value as PublishAction)} className={cn("flex w-full items-center justify-between rounded-[16px] border px-3 py-3 text-left text-sm transition-colors", publishAction === value ? "border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.08] text-foreground" : "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] text-muted-foreground hover:text-foreground")}>
                          <span>{label}</span>
                          <span className="text-[11px] uppercase tracking-[0.08em]">{publishAction === value ? t("experimentLab.selected", "已选") : ""}</span>
                        </button>
                      ))}
                    </div>
                  </div>
                </div>

                {selectedDraft.publish_readiness?.issues?.length ? (
                  <div className="rounded-[18px] border border-[hsl(var(--status-error-border))] bg-[hsl(var(--status-error-bg))] px-4 py-3 text-sm leading-6 text-[hsl(var(--status-error-text))]">
                    {t("experimentLab.publishValidationFailed", "发布前校验未通过：")}
                    {readinessMessagesLabel(selectedDraft.publish_readiness.issues, translate)}
                  </div>
                ) : null}

                {selectedDraft.publish_readiness?.warnings?.length ? (
                  <div className="rounded-[18px] border border-[hsl(var(--status-info-border))] bg-[hsl(var(--status-info-bg))] px-4 py-3 text-sm leading-6 text-[hsl(var(--status-info-text))]">
                    {t("experimentLab.validationWarnings", "当前验证提示：")}
                    {readinessMessagesLabel(selectedDraft.publish_readiness.warnings, translate)}
                  </div>
                ) : null}

                {publishAction === "schedule" ? (
                  <div className="space-y-2">
                    <label className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.cronLabel", "周期规则")}</label>
                    <Input value={scheduleCron} onChange={(e) => setScheduleCron(e.target.value)} placeholder={t("experimentLab.cronPlaceholder", "cron，例如 0 9 * * *")} />
                  </div>
                ) : null}

                {publishAction === "monitor" ? (
                  <div className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]">
                    <div className="space-y-2">
                      <label className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.pollIntervalLabel", "轮询秒数")}</label>
                      <Input value={monitorInterval} onChange={(e) => setMonitorInterval(e.target.value)} placeholder={t("experimentLab.pollIntervalPlaceholder", "轮询秒数，例如 300")} />
                    </div>
                    <div className="space-y-2">
                      <label className="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.monitorInstructionLabel", "监控说明")}</label>
                      <Textarea value={monitorInstruction} onChange={(e) => setMonitorInstruction(e.target.value)} className="min-h-[120px]" placeholder={t("experimentLab.monitorInstructionPlaceholder", "描述需要持续观察的条件、阈值和触发规则。")} />
                    </div>
                  </div>
                ) : null}

                <div className="flex flex-wrap gap-2">
                  <Button variant="outline" onClick={() => setBuilderStage("chat")}>{t("experimentLab.backToBuilder", "返回构建")}</Button>
                  <Button onClick={() => setPublishConfirmOpen(true)} disabled={!canManage || !selectedDraft?.publish_readiness?.ready || !publishModuleName.trim() || publishing}>
                    <Rocket className="mr-1 h-4 w-4" />
                    {publishing ? t("experimentLab.publishing", "发布中…") : t("experimentLab.publishApp", "发布应用")}
                  </Button>
                </div>
              </>
            )}
          </CardContent>
        </Card>
          );
        }
        return (
          <div className="space-y-4">
            <Card className="ui-section-panel">
              <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4">
                <div>
                  <div className="text-sm font-semibold text-foreground">{t("experimentLab.observeRuns", "观察运行")}</div>
                  <div className="mt-1 text-xs text-muted-foreground">{t("experimentLab.observeRunsDescription", "这里统一查看应用、运行、计划和产物，并给每次动作一个明确的查看入口。")}</div>
                </div>
                <div className="flex flex-wrap gap-2">
                  {OPS_TABS.map((tab) => (
                    <button
                      key={tab.key}
                      type="button"
                      onClick={() => setOpsTab(tab.key)}
                      className={cn(
                        "rounded-full border px-3 py-2 text-sm font-medium transition-colors",
                        opsTab === tab.key
                          ? "border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.08] text-foreground"
                          : "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] text-muted-foreground",
                      )}
                    >
                      {t(tab.labelKey, tab.fallback)}
                    </button>
                  ))}
                </div>
              </CardContent>
            </Card>

            {opsTab === "modules"
              ? modules.length > 0
                ? (
                    <div className="grid gap-3">
                      {modules.map((module) => {
                        const moduleIsRunning =
                          runningModuleAction?.moduleId === module.module_id &&
                          runningModuleAction.mode === "run_once";
                        const moduleLatestRun = latestRunByModule.get(module.module_id) || null;
                        return (
                          <Card key={module.module_id} className="ui-section-panel">
                            <CardContent className="flex flex-col gap-4 p-4 lg:flex-row lg:items-center lg:justify-between">
                              <div>
                                <div className="text-sm font-semibold text-foreground">{module.name}</div>
                                <div className="mt-1 flex flex-wrap gap-2 text-xs text-muted-foreground">
                                  <span>v{module.version}</span>
                                  <span>{t("experimentLab.systemCount", "{{count}} 个系统", { count: module.summary?.integration_count || module.integration_ids.length })}</span>
                                  {module.summary?.native_execution_ready ? <span>{t("experimentLab.nativeExecution", "原生执行")}</span> : null}
                                  {verificationLabels(module.summary, translate).map((label) => <span key={label}>{label}</span>)}
                                </div>
                                {moduleLatestRun ? (
                                  <div className="mt-2 text-xs text-muted-foreground">
                                    {t("experimentLab.latestRunPrefix", "Latest run: ")}{runModeLabel(moduleLatestRun.mode, translate)} · {runStatusLabel(moduleLatestRun.status, translate)} · {formatTime(moduleLatestRun.created_at)}
                                  </div>
                                ) : null}
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button size="sm" onClick={() => void handleRunModule(module, "chat")}>
                                  {t("experimentLab.openApp", "打开应用")}
                                </Button>
                                <Button variant="outline" size="sm" onClick={() => void handleRunModule(module, "run_once")} disabled={Boolean(runningModuleAction)}>
                                  {moduleIsRunning ? t("experimentLab.running", "运行中…") : t("experimentLab.runMode.runOnce", "一次运行")}
                                </Button>
                                {moduleLatestRun ? (
                                  <Button variant="outline" size="sm" onClick={() => openRunDetail(moduleLatestRun.run_id)}>
                                    {t("experimentLab.viewResult", "查看结果")}
                                  </Button>
                                ) : null}
                              </div>
                            </CardContent>
                          </Card>
                        );
                      })}
                    </div>
                  )
                : (
                    <Card className="ui-section-panel">
                      <CardContent className="p-8">
                        <EmptyState icon={<Beaker className="h-5 w-5" />} message={t("experimentLab.noPublishedApps", "还没有已发布 Agent。")} />
                      </CardContent>
                    </Card>
                  )
              : null}

            {opsTab === "runs"
              ? runs.length > 0
                ? (
                    <div className="grid gap-3">
                      {runs.map((run) => {
                        const expanded = expandedRunIds.includes(run.run_id);
                        const summary = run.summary || "";
                        const preview = expanded ? summary : excerptText(summary, 220);
                        return (
                          <Card key={run.run_id} className="ui-section-panel">
                            <CardContent className="flex flex-col gap-3 p-4 lg:flex-row lg:items-start lg:justify-between">
                              <div className="min-w-0 flex-1">
                                <div className="text-sm font-semibold text-foreground">{runModeLabel(run.mode, (key, fallback) => t(key, fallback))}</div>
                                <div className="mt-1 text-xs text-muted-foreground">
                                  {formatTime(run.created_at)} · {runStatusLabel(run.status, translate)}
                                  {run.session_id ? t("experimentLab.chatSessionRun", " · 对话会话") : t("experimentLab.nativeExecutionRun", " · 原生执行")}
                                </div>
                                {summary ? (
                                  <div className="mt-2 text-sm leading-6 text-muted-foreground">{preview}</div>
                                ) : (
                                  <div className="mt-2 text-sm leading-6 text-muted-foreground">{t("experimentLab.noRunSummary", "这次运行还没有可展示的摘要。")}</div>
                                )}
                                {summary && summary.length > 220 ? (
                                  <button type="button" className="mt-2 text-xs font-medium text-foreground" onClick={() => toggleExpandedRun(run.run_id)}>
                                    {expanded ? t("experimentLab.collapseSummary", "收起摘要") : t("experimentLab.expandSummary", "展开摘要")}
                                  </button>
                                ) : null}
                              </div>
                              <div className="flex flex-wrap gap-2">
                                <Button size="sm" onClick={() => openRunDetail(run.run_id)}>
                                  {t("experimentLab.viewResult", "查看结果")}
                                </Button>
                                {run.session_id ? (
                                  <Button variant="outline" size="sm" onClick={() => openRunConversation(run)}>
                                    {t("experimentLab.viewConversation", "查看对话")}
                                  </Button>
                                ) : null}
                              </div>
                            </CardContent>
                          </Card>
                        );
                      })}
                    </div>
                  )
                : (
                    <Card className="ui-section-panel">
                      <CardContent className="p-8">
                        <EmptyState icon={<Rocket className="h-5 w-5" />} message={t("experimentLab.noRuns", "还没有运行记录。")} />
                      </CardContent>
                    </Card>
                  )
              : null}

            {opsTab === "plans" ? (
              <div className="grid gap-4 xl:grid-cols-[minmax(0,320px)_minmax(0,1fr)]">
                <Card className="ui-section-panel">
                  <CardHeader>
                    <CardTitle className="text-base">{t("experimentLab.createPlan", "创建计划")}</CardTitle>
                  </CardHeader>
                  <CardContent className="space-y-3">
                    <Select value={selectedModule?.module_id || ""} onValueChange={(value) => setSelectedModuleId(value)}>
                      <SelectTrigger><SelectValue placeholder={t("experimentLab.selectModule", "选择模块")} /></SelectTrigger>
                      <SelectContent>
                        {modules.map((module) => (
                          <SelectItem key={module.module_id} value={module.module_id}>
                            {module.name} · v{module.version}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <div className="flex gap-2">
                      <Button className="flex-1" onClick={() => void handleCreateSchedule("schedule")} disabled={!canManage || !selectedModule || Boolean(creatingPlanMode)}>
                        {creatingPlanMode === "schedule" ? t("experimentLab.creating", "创建中…") : t("experimentLab.scheduleMode.schedule", "周期运行")}
                      </Button>
                      <Button variant="outline" className="flex-1" onClick={() => void handleCreateSchedule("monitor")} disabled={!canManage || !selectedModule || Boolean(creatingPlanMode)}>
                        {creatingPlanMode === "monitor" ? t("experimentLab.creating", "创建中…") : t("experimentLab.scheduleMode.monitor", "长期监控")}
                      </Button>
                    </div>
                    <Input value={scheduleCron} onChange={(e) => setScheduleCron(e.target.value)} placeholder={t("experimentLab.cronPlaceholder", "cron，例如 0 9 * * *")} />
                    <Input value={monitorInterval} onChange={(e) => setMonitorInterval(e.target.value)} placeholder={t("experimentLab.monitorPollPlaceholder", "监控轮询秒数，例如 300")} />
                    <Textarea value={monitorInstruction} onChange={(e) => setMonitorInstruction(e.target.value)} className="min-h-[100px]" placeholder={t("experimentLab.monitorInstructionOptional", "监控说明（可选）")} />
                  </CardContent>
                </Card>
                <div className="space-y-3">
                  {schedules.length > 0 ? schedules.map((schedule) => (
                    <Card key={schedule.schedule_id} className="ui-section-panel">
                      <CardContent className="flex flex-col gap-3 p-4 lg:flex-row lg:items-center lg:justify-between">
                        <div>
                          <div className="text-sm font-semibold text-foreground">{scheduleModeLabel(schedule.mode, (key, fallback) => t(key, fallback))}</div>
                          <div className="mt-1 text-xs text-muted-foreground">
                            {schedule.mode === "monitor" ? t("experimentLab.pollEverySeconds", "{{count}}s 轮询", { count: schedule.poll_interval_seconds || 300 }) : schedule.cron_expression}
                          </div>
                          <div className="mt-1 text-xs text-muted-foreground">{t("experimentLab.nextRun", "Next run: {{time}}", { time: formatTime(schedule.next_run_at) })}</div>
                        </div>
                        <div className="flex gap-2">
                          {schedule.last_run_id ? (
                            <Button variant="outline" size="sm" onClick={() => openRunDetail(schedule.last_run_id || "")}>
                              {t("experimentLab.viewLatestRun", "查看最近运行")}
                            </Button>
                          ) : null}
                          <Button variant="outline" size="sm" onClick={() => void handleToggleSchedule(schedule)} disabled={!canManage || Boolean(togglingScheduleId || deletingScheduleId)}>
                            {togglingScheduleId === schedule.schedule_id ? t("experimentLab.processing", "处理中…") : schedule.status === "active" ? t("experimentLab.pause", "暂停") : t("experimentLab.enable", "启用")}
                          </Button>
                          <Button variant="ghost" size="sm" onClick={() => void handleDeleteSchedule(schedule.schedule_id)} disabled={!canManage || Boolean(togglingScheduleId || deletingScheduleId)}>
                            {deletingScheduleId === schedule.schedule_id ? t("common.deleting", "删除中…") : t("common.delete", "删除")}
                          </Button>
                        </div>
                      </CardContent>
                    </Card>
                  )) : (
                    <Card className="ui-section-panel">
                      <CardContent className="p-8">
                        <EmptyState icon={<Plus className="h-5 w-5" />} message={t("experimentLab.noPlans", "还没有计划。")} />
                      </CardContent>
                    </Card>
                  )}
                </div>
              </div>
            ) : null}

            {opsTab === "artifacts"
              ? artifacts.length > 0
                ? (
                    <div className="grid gap-3">
                      {artifacts.map((artifact) => {
                        const expanded = expandedArtifactIds.includes(artifact.artifact_id);
                        const body = artifact.text_content || "";
                        const preview = expanded ? body : excerptText(body, 260);
                        return (
                          <Card key={artifact.artifact_id} className="ui-section-panel">
                            <CardContent className="p-4">
                              <div className="text-sm font-semibold text-foreground">{artifact.name}</div>
                              <div className="mt-1 text-xs text-muted-foreground">{artifact.kind} · {formatTime(artifact.created_at)}</div>
                              {body ? <div className="mt-3 rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.72] px-4 py-3 text-[12px] leading-6 text-muted-foreground">{preview}</div> : null}
                              {body && body.length > 260 ? (
                                <button type="button" className="mt-2 text-xs font-medium text-foreground" onClick={() => toggleExpandedArtifact(artifact.artifact_id)}>
                                  {expanded ? t("experimentLab.collapseContent", "收起内容") : t("experimentLab.expandContent", "展开内容")}
                                </button>
                              ) : null}
                              <div className="mt-3 flex flex-wrap gap-2">
                                <Button variant="outline" size="sm" onClick={() => openRunDetail(artifact.run_id)}>
                                  {t("experimentLab.relatedRun", "关联运行")}
                                </Button>
                              </div>
                            </CardContent>
                          </Card>
                        );
                      })}
                    </div>
                  )
                : (
                    <Card className="ui-section-panel">
                      <CardContent className="p-8">
                        <EmptyState icon={<FileText className="h-5 w-5" />} message={t("experimentLab.noArtifacts", "还没有产物。")} />
                      </CardContent>
                    </Card>
                  )
              : null}
          </div>
        );
      })()}

      {error ? <div className="rounded-2xl border border-[hsl(var(--status-error-border))] bg-[hsl(var(--status-error-bg))] px-4 py-3 text-sm text-[hsl(var(--status-error-text))]">{error}</div> : null}

      <ConfirmDialog
        open={publishConfirmOpen}
        onOpenChange={setPublishConfirmOpen}
        title={t("experimentLab.confirmPublishTitle", "确认发布这个 Agent 应用？")}
        description={t("experimentLab.confirmPublishDescription", "发布会生成一个新的应用版本。为避免重复保存，请先确认名称和发布后的第一步动作。")}
        confirmText={publishing ? t("experimentLab.publishing", "发布中…") : t("experimentLab.confirmPublish", "确认发布")}
        cancelText={t("experimentLab.checkAgain", "再检查一下")}
        onConfirm={() => void handlePublish()}
        loading={publishing}
      >
        <div className="space-y-3 text-sm text-muted-foreground">
          <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.72] px-4 py-3">
            <div className="text-xs uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.appName", "应用名称")}</div>
            <div className="mt-1 text-sm font-medium text-foreground">{publishModuleName || selectedDraft?.name || t("experimentLab.unnamedApp", "未命名应用")}</div>
          </div>
          <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.72] px-4 py-3">
            <div className="text-xs uppercase tracking-[0.08em] text-muted-foreground">{t("experimentLab.postPublishAction", "发布后动作")}</div>
            <div className="mt-1 text-sm font-medium text-foreground">
              {publishAction === "none"
                ? t("experimentLab.publishAction.none", "先保存下来")
                : publishAction === "chat"
                  ? t("experimentLab.publishAction.chat", "马上对话试用")
                  : publishAction === "run_once"
                    ? t("experimentLab.publishAction.runOnce", "先运行一次")
                    : publishAction === "schedule"
                      ? t("experimentLab.publishAction.schedule", "按周期运行")
                      : t("experimentLab.publishAction.monitor", "持续观察并运行")}
            </div>
          </div>
        </div>
      </ConfirmDialog>

      <ConfirmDialog
        open={Boolean(deleteModuleTarget)}
        onOpenChange={(open) => {
          if (!open) setDeleteModuleTarget(null);
        }}
        title={t("experimentLab.confirmDeleteAppTitle", "确认删除这个已发布 Agent？")}
        description={t("experimentLab.confirmDeleteAppDescription", "这会删除当前应用版本，以及关联的运行记录、计划和产物。用于清理测试发布项，不可恢复。")}
        confirmText={
          deletingModuleId === deleteModuleTarget?.module_id ? t("common.deleting", "删除中…") : t("experimentLab.confirmDelete", "确认删除")
        }
        cancelText={t("common.cancel", "取消")}
        onConfirm={() => void handleDeleteModule()}
        loading={Boolean(deleteModuleTarget && deletingModuleId === deleteModuleTarget.module_id)}
      />

      <ConfirmDialog
        open={Boolean(deleteProjectTarget)}
        onOpenChange={(open) => {
          if (!open) setDeleteProjectTarget(null);
        }}
        title={t("experimentLab.confirmDeleteProjectTitle", "确认删除这个项目？")}
        description={t("experimentLab.confirmDeleteProjectDescription", "这会删除项目本身，以及项目下的资料、草稿、已发布 Agent、运行记录、计划和关联会话。用于清理测试项目，不可恢复。")}
        confirmText={
          deletingProjectId === deleteProjectTarget?.project_id ? t("common.deleting", "删除中…") : t("experimentLab.confirmDelete", "确认删除")
        }
        cancelText={t("common.cancel", "取消")}
        onConfirm={() => void handleDeleteProject()}
        loading={Boolean(deleteProjectTarget && deletingProjectId === deleteProjectTarget.project_id)}
      />

      <BottomSheetPanel open={workspaceOpen} onOpenChange={setWorkspaceOpen} title={t("teamNav.workspace", "工作区")} description={t("experimentLab.workspaceDescription", "项目是轻量容器。先切项目，再决定默认 Agent。")}>
        <div className="space-y-5">
          <div className="space-y-2">
            {projects.map((project) => (
              <div
                key={project.project_id}
                className={cn(
                  "rounded-[18px] border px-4 py-4 transition-colors",
                  selectedProjectId === project.project_id
                    ? "border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.08]"
                    : "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88]",
                )}
              >
                <button
                  type="button"
                  onClick={() => {
                    setSelectedProjectId(project.project_id);
                    setWorkspaceOpen(false);
                    setBuilderStage("chat");
                    setSurface("builder");
                  }}
                  className="w-full text-left"
                >
                  <div className="text-sm font-semibold text-foreground">{project.name}</div>
                  <div className="mt-1 text-xs leading-5 text-muted-foreground">
                    {project.description || t("experimentLab.sharedProjectFallback", "团队内复用项目")}
                  </div>
                </button>
                {canManage ? (
                  <div className="mt-3 flex justify-end">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDeleteProjectTarget(project)}
                      disabled={Boolean(deletingProjectId)}
                    >
                      <Trash2 className="mr-1 h-4 w-4" />
                      {t("experimentLab.deleteProject", "删除项目")}
                    </Button>
                  </div>
                ) : null}
              </div>
            ))}
          </div>
          <Select value={defaultBuilderAgentId} onValueChange={setDefaultBuilderAgentId}>
            <SelectTrigger><SelectValue placeholder={t("experimentLab.selectDriverAgent", "选择驱动 Agent")} /></SelectTrigger>
            <SelectContent>{agents.map((agent) => <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>)}</SelectContent>
          </Select>
          <Input value={projectName} onChange={(e) => setProjectName(e.target.value)} placeholder={t("experimentLab.projectName", "项目名称")} />
          <Textarea value={projectDescription} onChange={(e) => setProjectDescription(e.target.value)} className="min-h-[96px]" placeholder={t("experimentLab.projectDescriptionOptional", "项目说明（可选）")} />
          <Button className="w-full" onClick={() => void handleCreateProject()} disabled={!canManage || !projectName.trim()}><Plus className="mr-1 h-4 w-4" />{t("experimentLab.createProject", "新建项目")}</Button>
        </div>
      </BottomSheetPanel>

      <BottomSheetPanel open={sourcesOpen} onOpenChange={setSourcesOpen} title={t("experimentLab.sourcesAndConnections", "资料与连接")} description={t("experimentLab.sourcesPanelDescription", "把 API 说明、链接和连接信息交给 agent。名称会自动推断，只有连接边界需要你补充。")} fullHeight>
        <div className="space-y-5">
          <div className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.6] px-4 py-4">
            <div className="text-sm font-medium text-foreground">{t("experimentLab.sourcesIntroTitle", "先给说明，再补连接")}</div>
            <div className="mt-1 text-xs leading-5 text-muted-foreground">
              {t("experimentLab.sourcesIntroDescription", "最小资料可以只是 OpenAPI、Postman、curl、接口链接或一段 Markdown 说明。Base URL 和认证方式只在需要真实测试或执行时再补。")}
            </div>
          </div>
          <div className="space-y-2">
            {integrations.map((integration) => (
              <label key={integration.integration_id} className="flex items-start gap-3 rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] px-4 py-3">
                <input type="checkbox" className="mt-1" checked={selectedIntegrationIds.includes(integration.integration_id)} onChange={(event) => setSelectedIntegrationIds((current) => event.target.checked ? Array.from(new Set([...current, integration.integration_id])) : current.filter((item) => item !== integration.integration_id))} />
                <div className="min-w-0"><div className="text-sm font-semibold text-foreground">{integration.name}</div><div className="mt-1 text-xs text-muted-foreground">{integration.base_url || t("experimentLab.baseUrlUnset", "base_url not set")} · {connectionStatusLabel(integration.connection_status, translate)}</div></div>
              </label>
            ))}
          </div>
          <Input value={integrationName} onChange={(e) => setIntegrationName(e.target.value)} placeholder={t("experimentLab.sourceNamePlaceholder", "资料名称（可留空，系统会自动推断）")} />
          <Select value={integrationSpecKind} onValueChange={(value) => setIntegrationSpecKind(value as IntegrationSpecKind)}>
            <SelectTrigger><SelectValue placeholder={t("experimentLab.specFormat", "说明格式")} /></SelectTrigger>
            <SelectContent>{["openapi", "postman", "curl", "markdown", "json", "text"].map((item) => <SelectItem key={item} value={item}>{item}</SelectItem>)}</SelectContent>
          </Select>
          <div className="flex items-center gap-2">
            <Button variant="outline" type="button" onClick={() => fileInputRef.current?.click()}>{t("experimentLab.importFile", "导入文件")}</Button>
            <span className="truncate text-xs text-muted-foreground">{integrationSourceFileName || t("experimentLab.supportedFileTypes", "支持 .json / .yaml / .md / .txt")}</span>
            <input ref={fileInputRef} type="file" className="hidden" accept=".json,.yaml,.yml,.md,.txt" onChange={(event) => { const file = event.target.files?.[0]; if (file) { void file.text().then((content) => { setIntegrationSpecContent(content); setIntegrationSourceFileName(file.name); }); } }} />
          </div>
          <Textarea value={integrationSpecContent} onChange={(e) => setIntegrationSpecContent(e.target.value)} className="min-h-[240px]" placeholder={t("experimentLab.specContentPlaceholder", "粘贴 OpenAPI / Postman / curl / Markdown API 说明，或者直接粘贴接口文档链接与关键说明")} />
          <details className="rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] px-4 py-4">
            <summary className="cursor-pointer text-sm font-medium text-foreground">{t("experimentLab.optionalConnectionInfo", "补充连接信息（可选）")}</summary>
            <div className="mt-4 space-y-3">
              <Input value={integrationBaseUrl} onChange={(e) => setIntegrationBaseUrl(e.target.value)} placeholder={t("experimentLab.baseUrlOptional", "Base URL（可选）")} />
              <Select value={integrationAuthType} onValueChange={(value) => setIntegrationAuthType(value as IntegrationAuthType)}>
                <SelectTrigger><SelectValue placeholder={t("experimentLab.authMode", "认证方式")} /></SelectTrigger>
                <SelectContent>{["none", "bearer", "header", "basic", "api_key"].map((item) => <SelectItem key={item} value={item}>{item}</SelectItem>)}</SelectContent>
              </Select>
              <Textarea value={integrationAuthConfig} onChange={(e) => setIntegrationAuthConfig(e.target.value)} className="min-h-[88px]" placeholder={t("experimentLab.authConfigJson", "认证配置 JSON")} />
            </div>
          </details>
          <Button className="w-full" onClick={() => void handleCreateIntegration()} disabled={!canManage || !selectedProjectId || !integrationSpecContent.trim()}>{t("experimentLab.saveSource", "保存资料")}</Button>
        </div>
      </BottomSheetPanel>

      <BottomSheetPanel open={contextOpen} onOpenChange={setContextOpen} title={t("experimentLab.builderDraft", "Builder 草稿")} description={t("experimentLab.builderDraftDescription", "查看当前草稿状态、切换其它草稿，或重新验证。")} fullHeight>
        {selectedDraft ? (
          <div className="space-y-5">
            <div className="rounded-[20px] border border-[hsl(var(--ui-line-soft))/0.72] px-4 py-4">
              <div className="flex items-center justify-between gap-3"><div><div className="text-sm font-semibold text-foreground">{selectedDraft.name}</div><div className="mt-1 text-xs text-muted-foreground">{formatTime(selectedDraft.updated_at)}</div></div><span className={cn("inline-flex items-center rounded-full px-2.5 py-1 text-[11px] font-semibold", draftTone(selectedDraft))}>{draftLabel(selectedDraft, translate)}</span></div>
              <div className="mt-3 text-sm leading-6 text-muted-foreground">{selectedDraft.goal}</div>
              <div className="mt-4 flex gap-2">
                <Button variant="outline" onClick={() => void handleProbeDraft()}><Zap className="mr-1 h-4 w-4" />{t("experimentLab.revalidate", "重新验证")}</Button>
                <Button onClick={selectedDraft.publish_readiness?.ready ? () => { setBuilderStage("publish"); setSurface("builder"); } : () => void handleProbeDraft()}>{selectedDraft.publish_readiness?.ready ? t("experimentLab.goPublish", "去发布") : t("experimentLab.recheck", "重新检查")}</Button>
              </div>
            </div>
            <div className="space-y-2">
              {drafts.map((draft) => (
                <button key={draft.draft_id} type="button" onClick={() => { setSelectedDraftId(draft.draft_id); setContextOpen(false); setBuilderStage("chat"); setSurface("builder"); }} className={cn("w-full rounded-[18px] border px-4 py-4 text-left transition-colors", selectedDraftId === draft.draft_id ? "border-[hsl(var(--primary))/0.22] bg-[hsl(var(--primary))/0.08]" : "border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88]")}>
                  <div className="text-sm font-semibold text-foreground">{draft.name}</div>
                  <div className="mt-1 text-xs leading-5 text-muted-foreground">{draft.goal}</div>
                </button>
              ))}
            </div>
          </div>
        ) : <EmptyState icon={<Sparkles className="h-5 w-5" />} message={t("experimentLab.noBuilderHistory", "还没有构建记录。发送第一条消息后会自动创建。")} />}
      </BottomSheetPanel>

      <BottomSheetPanel open={Boolean(runChatSessionId)} onOpenChange={(open) => { if (!open) { setRunChatSessionId(null); setRunChatAgentId(null); } }} title={t("experimentLab.runConversation", "运行对话")} description={t("experimentLab.runConversationDescription", "查看这次真实执行的会话轨迹和结果。")} fullHeight>
        {runChatSessionId && runChatAgentId ? (
          <div className="min-h-[60dvh]">
            <ChatConversation sessionId={runChatSessionId} agentId={runChatAgentId} agentName={agents.find((item) => item.id === runChatAgentId)?.name || runChatAgentId} agent={agents.find((item) => item.id === runChatAgentId) || undefined} teamId={teamId} headerVariant="compact" />
          </div>
        ) : null}
      </BottomSheetPanel>

      <BottomSheetPanel
        open={runDetailOpen}
        onOpenChange={(open) => {
          setRunDetailOpen(open);
          if (!open) setSelectedRunId(null);
        }}
        title={t("experimentLab.runDetailTitle", "运行详情")}
        description={t("experimentLab.runDetailDescription", "统一查看这次运行的状态、摘要和产物。")}
        fullHeight
      >
        {selectedRun ? (
          <div className="space-y-4 pb-2">
            <div className="rounded-[20px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-strong))/0.7] px-4 py-4">
              <div className="flex flex-wrap items-center gap-2">
                <div className="text-sm font-semibold text-foreground">{runModeLabel(selectedRun.mode, (key, fallback) => t(key, fallback))}</div>
                <span className="inline-flex items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] px-2.5 py-1 text-[11px] text-muted-foreground">{runStatusLabel(selectedRun.status, translate)}</span>
                <span className="inline-flex items-center rounded-full border border-[hsl(var(--ui-line-soft))/0.72] px-2.5 py-1 text-[11px] text-muted-foreground">{formatTime(selectedRun.created_at)}</span>
              </div>
              <div className="mt-3 text-sm leading-7 text-foreground">
                {selectedRun.summary || t("experimentLab.noGeneratedRunSummary", "当前还没有生成运行摘要。你可以先查看对话轨迹或下方产物。")}
              </div>
              <div className="mt-4 flex flex-wrap gap-2">
                {selectedRun.session_id ? (
                  <Button variant="outline" size="sm" onClick={() => openRunConversation(selectedRun)}>
                    {t("experimentLab.viewConversation", "查看对话")}
                  </Button>
                ) : null}
                <Button variant="outline" size="sm" onClick={() => { setSurface("ops"); setOpsTab("artifacts"); setRunDetailOpen(false); }}>
                  {t("experimentLab.viewAllArtifacts", "去看全部产物")}
                </Button>
              </div>
            </div>

            <div className="space-y-3">
              <div className="text-sm font-semibold text-foreground">{t("experimentLab.relatedArtifacts", "关联产物")}</div>
              {selectedRunArtifacts.length > 0 ? selectedRunArtifacts.map((artifact) => (
                <Card key={artifact.artifact_id} className="ui-section-panel">
                  <CardContent className="p-4">
                    <div className="text-sm font-semibold text-foreground">{artifact.name}</div>
                    <div className="mt-1 text-xs text-muted-foreground">{artifact.kind} · {formatTime(artifact.created_at)}</div>
                    {artifact.text_content ? (
                      <div className="mt-3 rounded-[18px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel))/0.88] px-4 py-3 text-[12px] leading-6 text-muted-foreground">
                        {excerptText(artifact.text_content, 420)}
                      </div>
                    ) : null}
                  </CardContent>
                </Card>
              )) : (
                <Card className="ui-section-panel">
                  <CardContent className="p-8">
                    <EmptyState icon={<FileText className="h-5 w-5" />} message={t("experimentLab.noArtifactsForRun", "这次运行暂时还没有产物。")} />
                  </CardContent>
                </Card>
              )}
            </div>
          </div>
        ) : (
          <EmptyState icon={<Rocket className="h-5 w-5" />} message={t("experimentLab.selectRunFirst", "先选择一条运行记录。")} />
        )}
      </BottomSheetPanel>
    </div>
  );
}
