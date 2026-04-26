import { useEffect, useMemo, useState } from "react";
import { Bot, Clock3, Pause, Play, Plus, RefreshCcw, Trash2 } from "lucide-react";

import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { Textarea } from "../ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../ui/select";
import { agentApi, type TeamAgent } from "../../api/agent";
import { chatApi, type ChatChannelMessage } from "../../api/chat";
import {
  scheduledTasksApi,
  type ScheduledTaskExecutionContract,
  type ScheduledTaskParseOverrides,
  type ScheduledTaskParseResult,
  type ScheduledTaskProfile,
  type ScheduledTaskRunOutcomeReason,
  type ScheduledTaskDetail,
  type ScheduledTaskDeliveryTier,
  type ScheduledTaskListView,
  type ScheduledTaskRepairEvent,
  type ScheduledTaskKind,
  type ScheduledTaskSummary,
} from "../../api/scheduledTasks";
import { ApiError } from "../../api/client";
import i18n from "../../i18n";

interface ScheduledTasksSectionProps {
  teamId: string;
  canManage: boolean;
}

interface TaskFormState {
  title: string;
  prompt: string;
  agentId: string;
  taskKind: ScheduledTaskKind;
  deliveryTier: ScheduledTaskDeliveryTier;
  oneShotAt: string;
  cronMode: CronMode;
  everyMinutes: string;
  everyHours: string;
  dailyTime: string;
  weeklyDays: string[];
  cronExpression: string;
  timezone: string;
}

type CronMode = "every_minutes" | "every_hours" | "daily_at" | "weekdays_at" | "weekly_on" | "custom";

interface CronBuilderState {
  mode: CronMode;
  everyMinutes: string;
  everyHours: string;
  dailyTime: string;
  weeklyDays: string[];
  cronExpression: string;
}

const RECOMMENDED_TIMEZONES = [
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Asia/Singapore",
  "UTC",
  "Europe/London",
  "Europe/Berlin",
  "America/New_York",
  "America/Los_Angeles",
];

const WEEKDAY_OPTIONS = [
  { value: "1", zh: "周一", en: "Mon" },
  { value: "2", zh: "周二", en: "Tue" },
  { value: "3", zh: "周三", en: "Wed" },
  { value: "4", zh: "周四", en: "Thu" },
  { value: "5", zh: "周五", en: "Fri" },
  { value: "6", zh: "周六", en: "Sat" },
  { value: "0", zh: "周日", en: "Sun" },
];

function bilingual(zh: string, en: string) {
  return i18n.language?.toLowerCase().startsWith("zh") ? zh : en;
}

function defaultTimezone() {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}

function availableTimezones() {
  const current = defaultTimezone();
  const intlWithSupportedValues = Intl as typeof Intl & {
    supportedValuesOf?: (key: string) => string[];
  };
  const dynamic =
    typeof intlWithSupportedValues.supportedValuesOf === "function"
      ? intlWithSupportedValues.supportedValuesOf("timeZone")
      : [];
  return Array.from(new Set([current, ...RECOMMENDED_TIMEZONES, ...dynamic]));
}

function emptyForm(agentId = ""): TaskFormState {
  const builder = parseCronExpression("0 9 * * *");
  return {
    title: "",
    prompt: "",
    agentId,
    taskKind: "one_shot",
    deliveryTier: "durable",
    oneShotAt: "",
    cronMode: builder.mode,
    everyMinutes: builder.everyMinutes,
    everyHours: builder.everyHours,
    dailyTime: builder.dailyTime,
    weeklyDays: builder.weeklyDays,
    cronExpression: builder.cronExpression,
    timezone: defaultTimezone(),
  };
}

function isoToLocalInput(value?: string | null) {
  if (!value) return "";
  const date = new Date(value);
  const offset = date.getTimezoneOffset();
  const local = new Date(date.getTime() - offset * 60_000);
  return local.toISOString().slice(0, 16);
}

function localInputToIso(value: string) {
  if (!value) return null;
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return null;
  }
  return date.toISOString();
}

function formatDateTimeDisplay(value?: string | null, timezone?: string) {
  if (!value) return bilingual("未设置", "Not set");
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  const locale = i18n.language?.toLowerCase().startsWith("zh") ? "zh-CN" : "en-US";
  return `${date.toLocaleString(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  })}${timezone ? ` (${timezone})` : ""}`;
}

function clampPositiveInteger(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return fallback;
  }
  return parsed;
}

function padTimePart(value: number) {
  return String(value).padStart(2, "0");
}

function normalizeDailyTime(value: string) {
  const match = /^(\d{1,2}):(\d{1,2})$/.exec(value.trim());
  if (!match) {
    return "09:00";
  }
  const hours = Math.min(23, Math.max(0, Number.parseInt(match[1], 10)));
  const minutes = Math.min(59, Math.max(0, Number.parseInt(match[2], 10)));
  return `${padTimePart(hours)}:${padTimePart(minutes)}`;
}

function normalizeWeeklyDays(days: string[]) {
  const allowed = new Set(WEEKDAY_OPTIONS.map((item) => item.value));
  const unique = Array.from(new Set(days.filter((item) => allowed.has(item))));
  const ordered = WEEKDAY_OPTIONS.map((item) => item.value).filter((item) => unique.includes(item));
  return ordered.length ? ordered : ["1"];
}

function weekdayLabels(days: string[]) {
  const ordered = normalizeWeeklyDays(days);
  const labels = WEEKDAY_OPTIONS.filter((item) => ordered.includes(item.value)).map((item) => bilingual(item.zh, item.en));
  return labels.join(bilingual("、", ", "));
}

function parseDayOfWeekField(value: string) {
  const normalized = value.trim();
  if (normalized === "1-5") {
    return { kind: "weekdays" as const, days: ["1", "2", "3", "4", "5"] };
  }
  if (/^\d(?:,\d)*$/.test(normalized)) {
    return { kind: "weekly" as const, days: normalizeWeeklyDays(normalized.split(",")) };
  }
  return null;
}

function parseCronExpression(value?: string | null): CronBuilderState {
  const normalized = (value || "").trim();
  const parts = normalized.split(/\s+/);
  if (parts.length === 5) {
    const [minute, hour, dayOfMonth, month, dayOfWeek] = parts;
    if (hour === "*" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*") {
      if (minute.startsWith("*/")) {
        const interval = clampPositiveInteger(minute.slice(2), 15);
        return {
          mode: "every_minutes",
          everyMinutes: String(interval),
          everyHours: "1",
          dailyTime: "09:00",
          weeklyDays: ["1"],
          cronExpression: normalized,
        };
      }
    }
      if (minute === "0" && dayOfMonth === "*" && month === "*" && dayOfWeek === "*") {
      if (hour === "*") {
        return {
          mode: "every_hours",
          everyMinutes: "15",
          everyHours: "1",
          dailyTime: "09:00",
          weeklyDays: ["1"],
          cronExpression: normalized,
        };
      }
      if (hour.startsWith("*/")) {
        const interval = clampPositiveInteger(hour.slice(2), 1);
        return {
          mode: "every_hours",
          everyMinutes: "15",
          everyHours: String(interval),
          dailyTime: "09:00",
          weeklyDays: ["1"],
          cronExpression: normalized,
        };
      }
      const dailyHours = Number.parseInt(hour, 10);
      const dailyMinutes = Number.parseInt(minute, 10);
      if (Number.isFinite(dailyHours) && Number.isFinite(dailyMinutes)) {
        return {
          mode: "daily_at",
          everyMinutes: "15",
          everyHours: "1",
          dailyTime: `${padTimePart(dailyHours)}:${padTimePart(dailyMinutes)}`,
          weeklyDays: ["1"],
          cronExpression: normalized,
        };
      }
    }
    if (
      dayOfMonth === "*" &&
      month === "*" &&
      /^\d+$/.test(minute) &&
      /^\d+$/.test(hour)
    ) {
      const parsedDayOfWeek = parseDayOfWeekField(dayOfWeek);
      if (parsedDayOfWeek?.kind === "weekdays") {
        return {
          mode: "weekdays_at",
          everyMinutes: "15",
          everyHours: "1",
          dailyTime: `${padTimePart(Number.parseInt(hour, 10))}:${padTimePart(Number.parseInt(minute, 10))}`,
          weeklyDays: parsedDayOfWeek.days,
          cronExpression: normalized,
        };
      }
      if (parsedDayOfWeek?.kind === "weekly") {
        return {
          mode: "weekly_on",
          everyMinutes: "15",
          everyHours: "1",
          dailyTime: `${padTimePart(Number.parseInt(hour, 10))}:${padTimePart(Number.parseInt(minute, 10))}`,
          weeklyDays: parsedDayOfWeek.days,
          cronExpression: normalized,
        };
      }
    }
    if (
      dayOfMonth === "*" &&
      month === "*" &&
      dayOfWeek === "*" &&
      /^\d+$/.test(minute) &&
      /^\d+$/.test(hour)
    ) {
      return {
        mode: "daily_at",
        everyMinutes: "15",
        everyHours: "1",
        dailyTime: `${padTimePart(Number.parseInt(hour, 10))}:${padTimePart(Number.parseInt(minute, 10))}`,
        weeklyDays: ["1"],
        cronExpression: normalized,
      };
    }
  }
  return {
    mode: "custom",
    everyMinutes: "15",
    everyHours: "1",
    dailyTime: "09:00",
    weeklyDays: ["1"],
    cronExpression: normalized || "0 9 * * *",
  };
}

function buildCronExpression(form: Pick<TaskFormState, "cronMode" | "everyMinutes" | "everyHours" | "dailyTime" | "weeklyDays" | "cronExpression">) {
  switch (form.cronMode) {
    case "every_minutes":
      return `*/${clampPositiveInteger(form.everyMinutes, 15)} * * * *`;
    case "every_hours": {
      const interval = clampPositiveInteger(form.everyHours, 1);
      return interval <= 1 ? "0 * * * *" : `0 */${interval} * * *`;
    }
    case "daily_at": {
      const [hoursRaw, minutesRaw] = normalizeDailyTime(form.dailyTime).split(":");
      return `${Number.parseInt(minutesRaw, 10)} ${Number.parseInt(hoursRaw, 10)} * * *`;
    }
    case "weekdays_at": {
      const [hoursRaw, minutesRaw] = normalizeDailyTime(form.dailyTime).split(":");
      return `${Number.parseInt(minutesRaw, 10)} ${Number.parseInt(hoursRaw, 10)} * * 1-5`;
    }
    case "weekly_on": {
      const [hoursRaw, minutesRaw] = normalizeDailyTime(form.dailyTime).split(":");
      return `${Number.parseInt(minutesRaw, 10)} ${Number.parseInt(hoursRaw, 10)} * * ${normalizeWeeklyDays(form.weeklyDays).join(",")}`;
    }
    case "custom":
    default:
      return form.cronExpression.trim();
  }
}

function scheduleSummaryFromForm(form: TaskFormState) {
  if (form.taskKind === "one_shot") {
    return `${bilingual("Run once:", "Run once:")} ${formatDateTimeDisplay(localInputToIso(form.oneShotAt), form.timezone)}`;
  }
  switch (form.cronMode) {
    case "every_minutes":
      return `${bilingual("Every", "Every")} ${clampPositiveInteger(form.everyMinutes, 15)} ${bilingual("minutes", "minutes")} (${form.timezone})`;
    case "every_hours":
      return `${bilingual("Every", "Every")} ${clampPositiveInteger(form.everyHours, 1)} ${bilingual("hours on the hour", "hours on the hour")} (${form.timezone})`;
    case "daily_at":
      return `${bilingual("Daily at", "Daily at")} ${normalizeDailyTime(form.dailyTime)} (${form.timezone})`;
    case "weekdays_at":
      return `${bilingual("Weekdays at", "Weekdays at")} ${normalizeDailyTime(form.dailyTime)} (${form.timezone})`;
    case "weekly_on":
      return `${bilingual("Weekly on", "Weekly on")} ${weekdayLabels(form.weeklyDays)} ${normalizeDailyTime(form.dailyTime)} (${form.timezone})`;
    case "custom":
    default:
      return `${bilingual("Custom schedule:", "Custom schedule:")} ${form.cronExpression || bilingual("未设置", "Not set")} (${form.timezone})`;
  }
}

function formatRepairNotice(events: ScheduledTaskRepairEvent[]) {
  if (!events.length) return null;
  const visibleTitles = events
    .map((event) => event.title.trim())
    .filter(Boolean)
    .slice(0, 3);
  if (events.length === 1) {
    return visibleTitles[0]
      ? `Task "${visibleTitles[0]}" referenced a missing channel and has been automatically cleaned up as deleted.`
      : events[0]?.user_message || null;
  }
  const suffix =
    visibleTitles.length === 0
      ? ""
      : `: ${visibleTitles.join(bilingual("、", ", "))}${events.length > visibleTitles.length ? bilingual(" 等", " and more") : ""}`;
  return `The system just cleaned up ${events.length} broken scheduled task(s)${suffix}. Their referenced channels no longer exist.`;
}

function deliveryTierLabel(value: ScheduledTaskDeliveryTier) {
  return value === "session_scoped" ? "Session-scoped" : "Durable";
}

function taskKindLabel(value: ScheduledTaskKind) {
  return value === "one_shot" ? "One-shot" : "Recurring";
}

function taskProfileLabel(value: ScheduledTaskProfile) {
  switch (value) {
    case "document_task":
      return "Document task";
    case "workspace_task":
      return "Workspace task";
    case "hybrid_task":
      return "Hybrid task";
    case "retrieval_task":
      return "External retrieval task";
    default:
      return value;
  }
}

function outputModeLabel(value: ScheduledTaskExecutionContract["output_mode"]) {
  return value === "summary_and_artifact" ? "Summary + artifact" : "Summary only";
}

function sourceScopeLabel(value: ScheduledTaskExecutionContract["source_scope"]) {
  switch (value) {
    case "team_documents":
      return "Team documents";
    case "workspace_only":
      return "Workspace only";
    case "mixed":
      return "Documents + workspace";
    case "external_retrieval":
      return "External retrieval";
    default:
      return value;
  }
}

function sourcePolicyLabel(value?: ScheduledTaskExecutionContract["source_policy"] | null) {
  switch (value) {
    case "official_first":
      return "Official first";
    case "domestic_preferred":
      return "Domestic sources first";
    case "global_preferred":
      return "Global sources first";
    case "mixed":
      return "Mixed domestic/global";
    default:
      return null;
  }
}

function publishBehaviorLabel(value: ScheduledTaskExecutionContract["publish_behavior"]) {
  switch (value) {
    case "publish_workspace_artifact":
      return "Publish workspace artifact";
    case "create_document_from_file":
      return "Create document from file";
    case "none":
    default:
      return "No write-back";
  }
}

function payloadKindLabel(value: ScheduledTaskSummary["payload_kind"]) {
  switch (value) {
    case "artifact_task":
      return "Artifact task";
    case "document_pipeline":
      return "Document pipeline";
    case "retrieval_pipeline":
      return "External retrieval pipeline";
    case "system_summary":
    default:
      return "System summary";
  }
}

function sessionBindingLabel(value: ScheduledTaskSummary["session_binding"]) {
  return value === "bound_session" ? "Bound session" : "Independent run";
}

function deliveryPlanLabel(value: ScheduledTaskSummary["delivery_plan"]) {
  switch (value) {
    case "channel_and_artifact":
      return "Channel + artifact";
    case "channel_and_publish":
      return "Channel + publish";
    case "channel_only":
    default:
      return "Channel only";
  }
}

function parseConfidenceLabel(value: number) {
  if (value >= 0.85) return "High";
  if (value >= 0.65) return "Medium";
  return "Needs review";
}

function outputContractSummary(contract: ScheduledTaskExecutionContract) {
  const mode = outputModeLabel(contract.output_mode);
  const source = sourceScopeLabel(contract.source_scope);
  const publish = publishBehaviorLabel(contract.publish_behavior);
  const policy = sourcePolicyLabel(contract.source_policy);
  return policy ? `${mode} · ${source} · ${policy} · ${publish}` : `${mode} · ${source} · ${publish}`;
}

function runOutcomeReasonLabel(value?: ScheduledTaskRunOutcomeReason | null) {
  switch (value) {
    case "completed":
      return "Completed";
    case "completed_with_warnings":
      return "Completed with warnings";
    case "failed_no_final_answer":
      return "Failed: no final answer";
    case "failed_contract_violation":
      return "Failed: execution contract not met";
    case "blocked_capability_policy":
      return "Blocked by capability policy";
    case "cancelled":
      return "Cancelled";
    default:
      return null;
  }
}

function runStatusLabel(value: string) {
  switch (value) {
    case "running":
      return "Running";
    case "completed":
      return "Completed";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    case "waiting_input":
      return "Waiting for input";
    case "waiting_approval":
      return "Waiting for approval";
    default:
      return value;
  }
}

function selfEvaluationGradeLabel(value?: import("../../api/scheduledTasks").ScheduledTaskSelfEvaluationGrade | null) {
  switch (value) {
    case "excellent":
      return "Excellent";
    case "good":
      return "Good";
    case "acceptable":
      return "Acceptable";
    case "weak":
      return "Weak";
    case "failed":
      return "Failed";
    default:
      return null;
  }
}

function isSelfEvaluationPending(run: ScheduledTaskDetail["runs"][number]) {
  if (run.self_evaluation || run.status === "running") {
    return false;
  }
  if (!run.finished_at) {
    return false;
  }
  const finished = new Date(run.finished_at).getTime();
  if (!Number.isFinite(finished)) {
    return false;
  }
  return Date.now() - finished < 90_000;
}

function taskStatusLabel(value: string) {
  switch (value) {
    case "draft":
      return "Draft";
    case "active":
      return "Active";
    case "paused":
      return "Paused";
    case "deleted":
      return "Deleted";
    default:
      return value;
  }
}

function triggerSourceLabel(value: string) {
  switch (value) {
    case "schedule":
      return "Scheduled";
    case "manual":
      return "Manual run now";
    case "missed_recovery":
      return "Missed-run recovery";
    default:
      return value;
  }
}

function isExpiredOneShotTask(task: ScheduledTaskDetail | null) {
  if (
    !task ||
    task.task_kind !== "one_shot" ||
    !["paused", "completed"].includes(task.status) ||
    !task.one_shot_at
  ) {
    return false;
  }
  const fireAt = new Date(task.one_shot_at);
  return !Number.isNaN(fireAt.getTime()) && fireAt.getTime() <= Date.now();
}

function isExpiredOneShotSummary(
  task: Pick<ScheduledTaskSummary, "task_kind" | "status" | "one_shot_at">,
) {
  if (
    task.task_kind !== "one_shot" ||
    !["paused", "completed"].includes(task.status) ||
    !task.one_shot_at
  ) {
    return false;
  }
  const fireAt = new Date(task.one_shot_at);
  return !Number.isNaN(fireAt.getTime()) && fireAt.getTime() <= Date.now();
}

function displayStatusLabelForSummary(task: ScheduledTaskSummary) {
  if (isExpiredOneShotSummary(task)) {
    return "Ended";
  }
  if (task.status === "completed") {
    return "Completed";
  }
  return taskStatusLabel(task.status);
}

function displayStatusLabelForDetail(task: ScheduledTaskDetail) {
  if (isExpiredOneShotTask(task)) {
    const latestRun = task.runs[0];
    switch (latestRun?.status) {
      case "completed":
        return "Completed";
      case "failed":
        return "Execution failed";
      case "cancelled":
        return "Cancelled";
      case "waiting_input":
        return "Waiting for additional input";
      case "waiting_approval":
        return "Waiting for approval";
      default:
        return "Ended";
    }
  }
  if (task.status === "completed") {
    return "Completed";
  }
  return taskStatusLabel(task.status);
}

function displayStatusToneForSummary(task: ScheduledTaskSummary) {
  if (isExpiredOneShotSummary(task)) {
    return "text-foreground";
  }
  if (task.status === "completed") {
    return "text-emerald-600";
  }
  return taskStatusTone(task.status);
}

function displayStatusToneForDetail(task: ScheduledTaskDetail) {
  if (isExpiredOneShotTask(task)) {
    const latestRun = task.runs[0];
    switch (latestRun?.status) {
      case "completed":
        return "text-emerald-600";
      case "failed":
        return "text-red-600";
      case "cancelled":
        return "text-amber-600";
      default:
        return "text-foreground";
    }
  }
  if (task.status === "completed") {
    return "text-emerald-600";
  }
  return taskStatusTone(task.status);
}

function explainRunError(
  error?: string | null,
  triggerSource?: string,
  outcomeReason?: ScheduledTaskRunOutcomeReason | null,
) {
  if (outcomeReason === "failed_contract_violation") {
    return {
      title: "This run did not satisfy the task contract",
      detail: "The task did not produce the required final result or artifact. Check the output requirements, artifact path, and publish settings.",
    };
  }
  if (outcomeReason === "blocked_capability_policy") {
    return {
      title: "This run was blocked by system capability policy",
      detail: "The requested execution capability is outside the allowed range of the current scheduled-task profile. Use a more suitable task type or adjust the task goal.",
    };
  }
  if (!error) return null;
  if (error.includes("runtime exited without a user-visible assistant response")) {
    return {
      title: "This run did not produce a displayable result",
      detail:
        triggerSource === "missed_recovery"
          ? "The system already triggered this run as a recovery, but the model still did not produce a final displayable reply. Make the prompt more explicit and require a fixed conclusion or format."
          : "The model did not produce a final displayable reply, so the task ended early. Make the prompt more explicit and require a fixed conclusion or format.",
    };
  }
  if (error.includes("cancel")) {
    return {
      title: "This run was cancelled",
      detail: "The task was cancelled before completion, so it did not produce a final result.",
    };
  }
  return {
    title: "This run failed",
    detail: error,
  };
}

function scheduleConfigPayload(form: TaskFormState) {
  if (form.taskKind !== "cron") {
    return null;
  }
  switch (form.cronMode) {
    case "every_minutes":
      return { mode: "every_minutes" as const, every_minutes: clampPositiveInteger(form.everyMinutes, 15), cron_expression: buildCronExpression(form) };
    case "every_hours":
      return { mode: "every_hours" as const, every_hours: clampPositiveInteger(form.everyHours, 1), cron_expression: buildCronExpression(form) };
    case "daily_at":
      return { mode: "daily_at" as const, daily_time: normalizeDailyTime(form.dailyTime), cron_expression: buildCronExpression(form) };
    case "weekdays_at":
      return {
        mode: "weekdays_at" as const,
        daily_time: normalizeDailyTime(form.dailyTime),
        weekly_days: ["1", "2", "3", "4", "5"],
        cron_expression: buildCronExpression(form),
      };
    case "weekly_on":
      return {
        mode: "weekly_on" as const,
        daily_time: normalizeDailyTime(form.dailyTime),
        weekly_days: normalizeWeeklyDays(form.weeklyDays),
        cron_expression: buildCronExpression(form),
      };
    case "custom":
    default:
      return { mode: "custom" as const, cron_expression: buildCronExpression(form) };
  }
}

function canReenableCompletedOneShot(task: ScheduledTaskDetail | null) {
  if (!task || task.task_kind !== "one_shot" || task.status !== "completed" || !task.one_shot_at) {
    return false;
  }
  const fireAt = new Date(task.one_shot_at);
  return !Number.isNaN(fireAt.getTime()) && fireAt.getTime() > Date.now();
}

function populateForm(task: ScheduledTaskDetail): TaskFormState {
  const builder = task.schedule_config
    ? {
        mode: task.schedule_config.mode,
        everyMinutes: String(task.schedule_config.every_minutes ?? 15),
        everyHours: String(task.schedule_config.every_hours ?? 1),
        dailyTime: task.schedule_config.daily_time || "09:00",
        weeklyDays: normalizeWeeklyDays(task.schedule_config.weekly_days || ["1"]),
        cronExpression: task.schedule_config.cron_expression || task.cron_expression || "0 9 * * *",
      }
    : parseCronExpression(task.cron_expression || "0 9 * * *");
  return {
    title: task.title,
    prompt: task.prompt,
    agentId: task.agent_id,
    taskKind: task.task_kind,
    deliveryTier: task.delivery_tier,
    oneShotAt: isoToLocalInput(task.one_shot_at),
    cronMode: builder.mode,
    everyMinutes: builder.everyMinutes,
    everyHours: builder.everyHours,
    dailyTime: builder.dailyTime,
    weeklyDays: builder.weeklyDays,
    cronExpression: builder.cronExpression,
    timezone: task.timezone || defaultTimezone(),
  };
}

function taskStatusTone(status: string) {
  switch (status) {
    case "active":
    case "completed":
      return "text-emerald-600";
    case "paused":
      return "text-amber-600";
    case "failed":
    case "deleted":
      return "text-red-600";
    default:
      return "text-muted-foreground";
  }
}

export function ScheduledTasksSection({ teamId, canManage }: ScheduledTasksSectionProps) {
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [tasks, setTasks] = useState<ScheduledTaskSummary[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [selectedTask, setSelectedTask] = useState<ScheduledTaskDetail | null>(null);
  const [messages, setMessages] = useState<ChatChannelMessage[]>([]);
  const [activeTab, setActiveTab] = useState<"create" | "overview" | "activity">("create");
  const [tabInitialized, setTabInitialized] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [listView, setListView] = useState<ScheduledTaskListView>("mine");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [parsing, setParsing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [form, setForm] = useState<TaskFormState>(emptyForm());
  const [quickText, setQuickText] = useState("");
  const [parsePreview, setParsePreview] = useState<ScheduledTaskParseResult | null>(null);
  const [previewAgentId, setPreviewAgentId] = useState("");
  const [previewTimezone, setPreviewTimezone] = useState(defaultTimezone());
  const [previewArtifactPath, setPreviewArtifactPath] = useState("");

  const selectedAgentName = useMemo(
    () =>
      agents.find((item) => item.id === (selectedTask?.agent_id || form.agentId))?.name ||
      selectedTask?.agent_id ||
      form.agentId,
    [agents, selectedTask?.agent_id, form.agentId],
  );
  const timezoneOptions = useMemo(() => availableTimezones(), []);
  const expiredOneShot = useMemo(() => isExpiredOneShotTask(selectedTask), [selectedTask]);
  const reenableCompletedOneShot = useMemo(
    () => canReenableCompletedOneShot(selectedTask),
    [selectedTask],
  );

  const loadTasks = async () => {
    const result = await scheduledTasksApi.list(teamId, listView);
    setTasks(result.tasks);
    setNotice(formatRepairNotice(result.repair_events || []));
    return result;
  };

  const loadTaskDetail = async (taskId: string) => {
    const detail = await scheduledTasksApi.get(teamId, taskId);
    setSelectedTask(detail);
    setForm(populateForm(detail));
    const rootMessages = await chatApi.listChannelMessages(detail.channel_id);
    setMessages(rootMessages);
    return detail;
  };

  const handleRecoveredTaskRemoval = async () => {
    const result = await loadTasks();
    const fallbackTaskId = result.tasks[0]?.task_id || null;
    setSelectedTaskId(fallbackTaskId);
    if (fallbackTaskId) {
      const detail = await scheduledTasksApi.get(teamId, fallbackTaskId);
      setSelectedTask(detail);
      setForm(populateForm(detail));
      const rootMessages = await chatApi.listChannelMessages(detail.channel_id);
      setMessages(rootMessages);
    } else {
      setSelectedTask(null);
      setMessages([]);
      setForm(emptyForm(agents[0]?.id || ""));
    }
  };

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      try {
        setLoading(true);
        const [agentRes, taskResult] = await Promise.all([
          agentApi.listAgents(teamId, 1, 200),
          scheduledTasksApi.list(teamId, listView),
        ]);
        if (cancelled) return;
        setAgents(agentRes.items || []);
        setTasks(taskResult.tasks);
        setNotice(formatRepairNotice(taskResult.repair_events || []));
        const initialTaskId =
          selectedTaskId && taskResult.tasks.some((item) => item.task_id === selectedTaskId)
            ? selectedTaskId
            : taskResult.tasks[0]?.task_id || null;
        setSelectedTaskId(initialTaskId);
        if (initialTaskId) {
          const detail = await scheduledTasksApi.get(teamId, initialTaskId);
          if (cancelled) return;
          setSelectedTask(detail);
          setForm(populateForm(detail));
          const rootMessages = await chatApi.listChannelMessages(detail.channel_id);
          if (cancelled) return;
          setMessages(rootMessages);
        } else {
          setSelectedTask(null);
          setForm(emptyForm(agentRes.items[0]?.id || ""));
          setMessages([]);
        }
        setError(null);
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to load scheduled tasks");
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };
    run();
    return () => {
      cancelled = true;
    };
  }, [teamId, listView]);

  useEffect(() => {
    if (!previewAgentId && agents[0]?.id) {
      setPreviewAgentId(agents[0].id);
    }
  }, [agents, previewAgentId]);

  useEffect(() => {
    setTabInitialized(false);
  }, [teamId]);

  useEffect(() => {
    if (!tabInitialized) {
      setActiveTab(tasks.length ? "overview" : "create");
      setTabInitialized(true);
    }
  }, [tasks.length, tabInitialized]);

  const selectTask = async (taskId: string) => {
    setSelectedTaskId(taskId);
    setLoading(true);
    try {
      await loadTaskDetail(taskId);
      setError(null);
    } catch (err) {
      if (err instanceof ApiError && err.status === 404) {
        await handleRecoveredTaskRemoval();
        setError(null);
        setNotice("The task you just selected referenced a missing channel and has been automatically cleaned up as deleted.");
      } else {
        setError(err instanceof Error ? err.message : "Failed to load task details");
      }
    } finally {
      setLoading(false);
    }
  };

  const reload = async (keepSelection = true) => {
    setLoading(true);
    try {
      const result = await loadTasks();
      const items = result.tasks;
      const targetId =
        keepSelection && selectedTaskId && items.some((item) => item.task_id === selectedTaskId)
          ? selectedTaskId
          : items[0]?.task_id || null;
      setSelectedTaskId(targetId);
      if (targetId) {
        await loadTaskDetail(targetId);
      } else {
        setSelectedTask(null);
        setMessages([]);
      }
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Refresh failed");
    } finally {
      setLoading(false);
    }
  };

  const resetQuickComposer = () => {
    setQuickText("");
    setParsePreview(null);
    setPreviewArtifactPath("");
    setPreviewTimezone(defaultTimezone());
    setPreviewAgentId(agents[0]?.id || "");
  };

  const handleParsePreview = async () => {
    if (!quickText.trim()) {
      setError("Describe the task in one sentence first.");
      return;
    }
    setParsing(true);
    try {
      const preview = await scheduledTasksApi.parse(teamId, {
        text: quickText.trim(),
        timezone: previewTimezone,
        agent_id: previewAgentId || undefined,
      });
      setParsePreview(preview);
      setPreviewArtifactPath(preview.execution_contract.artifact_path || "");
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to parse task");
    } finally {
      setParsing(false);
    }
  };

  const handleCreateFromPreview = async () => {
    if (!parsePreview) {
      setError("Generate a task preview first.");
      return;
    }
    if (!parsePreview.ready_to_create) {
      setError("This preview is not ready to create yet. Add a clear time description first.");
      return;
    }
    setSaving(true);
    try {
      const overrides: ScheduledTaskParseOverrides = {
        agent_id: previewAgentId || undefined,
        timezone: previewTimezone,
      };
      if (previewArtifactPath.trim()) {
        overrides.artifact_path = previewArtifactPath.trim();
      }
      const detail = await scheduledTasksApi.createFromParse(teamId, {
        preview: parsePreview,
        overrides,
      });
      await reload(false);
      setSelectedTaskId(detail.task_id);
      await loadTaskDetail(detail.task_id);
      setActiveTab("overview");
      setQuickText("");
      setParsePreview(null);
      setPreviewArtifactPath("");
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Creation failed");
    } finally {
      setSaving(false);
    }
  };

  const runAction = async (action: () => Promise<unknown>) => {
    setSaving(true);
    try {
      await action();
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Action failed");
    } finally {
      setSaving(false);
    }
  };

  const handleUpdate = async () => {
    if (!selectedTaskId) return;
    setSaving(true);
    try {
      const detail = await scheduledTasksApi.update(teamId, selectedTaskId, {
        agent_id: form.agentId,
        title: form.title.trim(),
        prompt: form.prompt.trim(),
        task_kind: form.taskKind,
        delivery_tier: form.deliveryTier,
        one_shot_at: form.taskKind === "one_shot" ? localInputToIso(form.oneShotAt) : null,
        cron_expression: form.taskKind === "cron" ? buildCronExpression(form) : null,
        schedule_config: scheduleConfigPayload(form),
        timezone: form.timezone.trim() || defaultTimezone(),
      });
      setSelectedTask(detail);
      setForm(populateForm(detail));
      setEditOpen(false);
      await reload();
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Save failed");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-4">
      <div className="flex items-center justify-between gap-3 border-b border-[hsl(var(--ui-line-soft))/0.72] pb-3">
        <div>
          <h2 className="text-xl font-semibold tracking-tight">{bilingual("定时任务", "Scheduled tasks")}</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {bilingual(
              "每个任务都是一个独立频道，触发记录和执行结果都沉淀在对应频道里。",
              "Each task is an independent channel. Trigger history and execution results are stored in its corresponding channel.",
            )}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Select value={listView} onValueChange={(value) => setListView(value as ScheduledTaskListView)}>
            <SelectTrigger className="w-[140px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="mine">{bilingual("我的任务", "My tasks")}</SelectItem>
              <SelectItem value="all_visible">{bilingual("全部可见", "All visible")}</SelectItem>
            </SelectContent>
          </Select>
          <Button variant="outline" size="sm" onClick={() => reload()}>
            <RefreshCcw className="mr-2 h-4 w-4" />
            {bilingual("刷新", "Refresh")}
          </Button>
          {canManage ? (
            <Button size="sm" onClick={resetQuickComposer}>
              <Plus className="mr-2 h-4 w-4" />
              {bilingual("新建任务", "New task")}
            </Button>
          ) : null}
        </div>
      </div>

      {error ? (
        <div className="rounded-lg border border-red-200 bg-red-50 px-4 py-3 text-sm text-red-700">
          {error}
        </div>
      ) : null}
      {notice ? (
        <div className="rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800">
          {notice}
        </div>
      ) : null}

      <Tabs
        value={activeTab}
        onValueChange={(value) => setActiveTab(value as "create" | "overview" | "activity")}
        className="flex min-h-0 flex-1 flex-col"
      >
        <TabsList className="w-full justify-start gap-6 border-b border-[hsl(var(--ui-line-soft))/0.72]">
          <TabsTrigger value="create">{bilingual("创建任务", "Create")}</TabsTrigger>
          <TabsTrigger value="overview">{bilingual("任务总览", "Overview")}</TabsTrigger>
          <TabsTrigger value="activity">{bilingual("运行与频道", "Runs & channel")}</TabsTrigger>
        </TabsList>

        <TabsContent value="create" className="min-h-0 flex-1">

      {canManage ? (
        <Card className="overflow-hidden border-[hsl(var(--ui-line-soft))/0.72] bg-[linear-gradient(135deg,rgba(15,23,42,0.02),rgba(59,130,246,0.08))]">
          <CardHeader className="pb-3">
            <CardTitle className="text-base">{bilingual("一句话创建任务", "Create from one sentence")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1.4fr)_280px]">
              <div className="space-y-3">
                <Textarea
                  value={quickText}
                  onChange={(event) => setQuickText(event.target.value)}
                  rows={4}
                  placeholder={bilingual(
                    "例如：每天早上 9 点读取团队最新文档变化，并生成一份 md 总结保存到工作区。",
                    "For example: every day at 9:00 read the latest team document changes and save an md summary into the workspace.",
                  )}
                />
                <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                  <span className="rounded-full border border-[hsl(var(--ui-line-soft))/0.72] px-3 py-1">
                    {bilingual("每天早上 9 点总结团队文档变化", "Daily 9:00 summary of team document changes")}
                  </span>
                  <span className="rounded-full border border-[hsl(var(--ui-line-soft))/0.72] px-3 py-1">
                    {bilingual("每周一 10 点生成项目进展 md", "Every Monday at 10:00 generate a project progress md")}
                  </span>
                  <span className="rounded-full border border-[hsl(var(--ui-line-soft))/0.72] px-3 py-1">
                    {bilingual("明天下午 3 点提醒我检查发布结果", "Remind me tomorrow at 3:00 PM to check the release result")}
                  </span>
                </div>
              </div>
              <div className="grid gap-3 rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-background/70 p-4">
                <div className="space-y-2">
                  <label className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                    {bilingual("执行 Agent", "Execution agent")}
                  </label>
                  <Select value={previewAgentId} onValueChange={setPreviewAgentId}>
                    <SelectTrigger>
                      <SelectValue placeholder={bilingual("选择 Agent", "Select agent")} />
                    </SelectTrigger>
                    <SelectContent>
                      {agents.map((agent) => (
                        <SelectItem key={agent.id} value={agent.id}>
                          {agent.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <label className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                    {bilingual("时区", "Time zone")}
                  </label>
                  <Input
                    list="scheduled-task-parse-timezone-options"
                    value={previewTimezone}
                    onChange={(event) => setPreviewTimezone(event.target.value)}
                    placeholder="Asia/Shanghai"
                  />
                  <datalist id="scheduled-task-parse-timezone-options">
                    {timezoneOptions.map((timezone) => (
                      <option key={timezone} value={timezone} />
                    ))}
                  </datalist>
                </div>
                <Button onClick={() => void handleParsePreview()} disabled={parsing}>
                  {parsing ? bilingual("解析中...", "Parsing...") : bilingual("生成任务预览", "Generate task preview")}
                </Button>
              </div>
            </div>

            {parsePreview ? (
              <div className="grid gap-4 rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-background/80 p-4 lg:grid-cols-[minmax(0,1.2fr)_320px]">
                <div className="space-y-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="rounded-full bg-primary/10 px-3 py-1 text-xs font-medium text-primary">
                      {parsePreview.human_schedule}
                    </span>
                    <span className="rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">
                      {taskProfileLabel(parsePreview.task_profile)}
                    </span>
                    <span className="rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">
                      {bilingual("置信度", "Confidence")}: {parseConfidenceLabel(parsePreview.confidence)}
                    </span>
                    {parsePreview.advanced_mode ? (
                      <span className="rounded-full bg-amber-100 px-3 py-1 text-xs font-medium text-amber-800">
                        {bilingual("高级模式", "Advanced mode")}
                      </span>
                    ) : null}
                  </div>
                  <div>
                    <div className="text-lg font-semibold tracking-tight">{parsePreview.title}</div>
                    <div className="mt-1 text-sm text-muted-foreground">{parsePreview.prompt}</div>
                  </div>
                  <div className="grid gap-2 rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-muted/20 p-3 text-sm text-muted-foreground">
                    <div>{bilingual("执行合同", "Execution contract")}: <span className="font-medium text-foreground">{outputContractSummary(parsePreview.execution_contract)}</span></div>
                    <div>{bilingual("结果去向", "Delivery plan")}: <span className="font-medium text-foreground">{parsePreview.delivery_plan === "channel_and_publish" ? bilingual("频道 + 发布文档", "Channel + published document") : parsePreview.delivery_plan === "channel_and_artifact" ? bilingual("频道 + 产物文件", "Channel + artifact file") : bilingual("仅频道", "Channel only")}</span></div>
                    <div>{bilingual("会话绑定", "Session binding")}: <span className="font-medium text-foreground">{parsePreview.session_binding === "bound_session" ? bilingual("绑定当前会话", "Bind current session") : bilingual("独立执行", "Independent run")}</span></div>
                  </div>
                  {parsePreview.warnings.length ? (
                    <div className="space-y-2">
                      {parsePreview.warnings.map((warning) => (
                        <div key={warning} className="rounded-xl border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-800">
                          {warning}
                        </div>
                      ))}
                    </div>
                  ) : null}
                </div>
                <div className="space-y-3 rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-muted/20 p-4">
                  {parsePreview.execution_contract.output_mode === "summary_and_artifact" ? (
                    <div className="space-y-2">
                      <label className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                        {bilingual("产物路径", "Artifact path")}
                      </label>
                      <Input
                        value={previewArtifactPath}
                        onChange={(event) => setPreviewArtifactPath(event.target.value)}
                        placeholder="reports/daily-digest.md"
                      />
                    </div>
                  ) : null}
                  <div className="text-xs text-muted-foreground">
                    {bilingual(
                      "这个预览已经包含调度、执行类型和输出合同。确认后会直接创建为可执行任务频道。",
                      "This preview already includes the schedule, execution type, and output contract. After confirmation it will be created directly as an executable task channel.",
                    )}
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button onClick={() => void handleCreateFromPreview()} disabled={saving || !parsePreview.ready_to_create}>
                      {saving ? bilingual("创建中...", "Creating...") : bilingual("确认并创建", "Confirm and create")}
                    </Button>
                    <Button variant="outline" onClick={resetQuickComposer} disabled={saving || parsing}>
                      {bilingual("重新开始", "Start over")}
                    </Button>
                  </div>
                </div>
              </div>
            ) : null}
          </CardContent>
        </Card>
      ) : (
        <Card>
          <CardContent className="px-5 py-8 text-sm text-muted-foreground">
            {bilingual(
              "当前账号没有管理权限，只能查看已有定时任务。",
              "Your current account does not have management permission and can only view existing scheduled tasks.",
            )}
          </CardContent>
        </Card>
      )}

        </TabsContent>

        <TabsContent value="overview" className="min-h-0 flex-1">

      <div className="grid min-h-0 flex-1 grid-cols-[320px_minmax(0,1fr)] gap-4">
        <Card className="min-h-0 overflow-hidden">
          <CardHeader className="pb-3">
            <CardTitle className="text-base">{bilingual("任务列表", "Task list")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2 overflow-y-auto">
            {tasks.map((task) => (
              <button
                key={task.task_id}
                type="button"
                onClick={() => void selectTask(task.task_id)}
                className={`w-full rounded-xl border px-3 py-3 text-left transition ${
                  selectedTaskId === task.task_id
                    ? "border-primary bg-primary/5"
                    : "border-border hover:border-primary/40 hover:bg-muted/40"
                }`}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-semibold">{task.title}</div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      {deliveryTierLabel(task.delivery_tier)} · {taskKindLabel(task.task_kind)} · {task.timezone}
                    </div>
                  </div>
                  <span className={`shrink-0 text-xs font-medium ${displayStatusToneForSummary(task)}`}>
                    {displayStatusLabelForSummary(task)}
                  </span>
                </div>
                <div className="mt-2 text-xs text-muted-foreground">
                  {bilingual("调度", "Schedule")}: {task.human_schedule}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">
                  {bilingual("下次触发", "Next run")}: {task.next_fire_at ? formatDateTimeDisplay(task.next_fire_at, task.timezone) : bilingual("未发布", "Not published")}
                </div>
              </button>
            ))}
            {!tasks.length && !loading ? (
              <div className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
                {bilingual("还没有定时任务。", "No scheduled tasks yet.")}
              </div>
            ) : null}
          </CardContent>
        </Card>

        <div className="min-h-0 overflow-y-auto space-y-4">
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-base">
                {selectedTask ? bilingual("任务详情", "Task details") : bilingual("选择一个任务", "Select a task")}
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              {!selectedTask ? (
                <div className="rounded-2xl border border-dashed border-[hsl(var(--ui-line-soft))/0.72] bg-muted/20 px-5 py-8 text-sm text-muted-foreground">
                  {bilingual(
                    "从上方“一句话创建任务”开始，系统会先解析出可执行预览；或者从左侧选择一个已有任务查看详情与运行记录。",
                    "Start from “Create from one sentence” above and the system will first generate an executable preview. Or select an existing task on the left to inspect details and run history.",
                  )}
                </div>
              ) : (
                <>
                  <div className="space-y-4">
                    <div className="rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-[linear-gradient(135deg,rgba(15,23,42,0.02),rgba(59,130,246,0.06))] px-5 py-4">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className={`text-sm font-semibold ${displayStatusToneForDetail(selectedTask)}`}>
                          {displayStatusLabelForDetail(selectedTask)}
                        </span>
                        <span className="rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">
                          {taskProfileLabel(selectedTask.task_profile)}
                        </span>
                        <span className="rounded-full bg-muted px-3 py-1 text-xs text-muted-foreground">
                          {deliveryTierLabel(selectedTask.delivery_tier)}
                        </span>
                        {selectedTask.task_profile === "retrieval_task" ? (
                          <span className="rounded-full bg-amber-100 px-3 py-1 text-xs font-medium text-amber-800">
                            {bilingual("高级模式", "Advanced mode")}
                          </span>
                        ) : null}
                      </div>
                      <div className="mt-3 text-lg font-semibold tracking-tight">{selectedTask.title}</div>
                      <div className="mt-2 text-sm leading-6 text-muted-foreground">{selectedTask.prompt}</div>
                    </div>

                    <div className="grid gap-4 md:grid-cols-2">
                      <div className="rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-muted/20 p-4">
                        <div className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                          {bilingual("执行摘要", "Execution summary")}
                        </div>
                        <div className="mt-3 space-y-2 text-sm text-muted-foreground">
                          <div>
                            {bilingual("执行 Agent", "Execution agent")}:
                            <span className="font-medium text-foreground"> {selectedAgentName}</span>
                          </div>
                          <div>
                            {bilingual("调度", "Schedule")}:
                            <span className="font-medium text-foreground"> {selectedTask.human_schedule}</span>
                          </div>
                          <div>
                            {bilingual("下次触发", "Next run")}:
                            <span className="font-medium text-foreground">
                              {" "}
                              {selectedTask.next_fire_at
                                ? formatDateTimeDisplay(selectedTask.next_fire_at, selectedTask.timezone)
                                : bilingual("未发布", "Not published")}
                            </span>
                          </div>
                          <div>
                            {bilingual("时区", "Time zone")}:
                            <span className="font-medium text-foreground"> {selectedTask.timezone}</span>
                          </div>
                          <div>
                            {bilingual("调度绑定", "Schedule binding")}:
                            <span className="font-medium text-foreground">
                              {" "}
                              {sessionBindingLabel(selectedTask.session_binding)}
                            </span>
                          </div>
                        </div>
                      </div>

                      <div className="rounded-2xl border border-[hsl(var(--ui-line-soft))/0.72] bg-muted/20 p-4">
                        <div className="text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
                          {bilingual("执行合同", "Execution contract")}
                        </div>
                        <div className="mt-3 space-y-2 text-sm text-muted-foreground">
                          <div>
                            {bilingual("输出方式", "Output mode")}:
                            <span className="font-medium text-foreground">
                              {" "}
                              {outputModeLabel(selectedTask.execution_contract.output_mode)}
                            </span>
                          </div>
                          <div>
                            {bilingual("数据范围", "Data scope")}:
                            <span className="font-medium text-foreground">
                              {" "}
                              {sourceScopeLabel(selectedTask.execution_contract.source_scope)}
                            </span>
                          </div>
                          {selectedTask.execution_contract.source_policy ? (
                            <div>
                              {bilingual("信息源策略", "Source policy")}:
                              <span className="font-medium text-foreground">
                                {" "}
                                {sourcePolicyLabel(selectedTask.execution_contract.source_policy)}
                              </span>
                            </div>
                          ) : null}
                          {selectedTask.task_profile === "retrieval_task" ? (
                            <>
                              {selectedTask.execution_contract.minimum_source_attempts ? (
                                <div>
                                  {bilingual("最少来源尝试", "Minimum source attempts")}:
                                  <span className="font-medium text-foreground">
                                    {" "}
                                    {selectedTask.execution_contract.minimum_source_attempts}
                                  </span>
                                </div>
                              ) : null}
                              {selectedTask.execution_contract.minimum_successful_sources ? (
                                <div>
                                  {bilingual("最少成功来源", "Minimum successful sources")}:
                                  <span className="font-medium text-foreground">
                                    {" "}
                                    {selectedTask.execution_contract.minimum_successful_sources}
                                  </span>
                                </div>
                              ) : null}
                              <div>
                                {bilingual("结构化源优先", "Prefer structured sources")}:
                                <span className="font-medium text-foreground">
                                  {" "}
                                  {selectedTask.execution_contract.prefer_structured_sources ? bilingual("是", "Yes") : bilingual("否", "No")}
                                </span>
                              </div>
                              <div>
                                {bilingual("允许查询重试", "Allow query retry")}:
                                <span className="font-medium text-foreground">
                                  {" "}
                                  {selectedTask.execution_contract.allow_query_retry ? bilingual("是", "Yes") : bilingual("否", "No")}
                                </span>
                              </div>
                              <div>
                                {bilingual("允许次级来源回退", "Allow secondary-source fallback")}:
                                <span className="font-medium text-foreground">
                                  {" "}
                                  {selectedTask.execution_contract.fallback_to_secondary_sources ? bilingual("是", "Yes") : bilingual("否", "No")}
                                </span>
                              </div>
                            </>
                          ) : null}
                          <div>
                            {bilingual("投递方式", "Delivery plan")}:
                            <span className="font-medium text-foreground">
                              {" "}
                              {deliveryPlanLabel(selectedTask.delivery_plan)}
                            </span>
                          </div>
                          <div>
                            Payload：
                            <span className="font-medium text-foreground">
                              {" "}
                              {payloadKindLabel(selectedTask.payload_kind)}
                            </span>
                          </div>
                          <div>
                            {bilingual("回写策略", "Write-back strategy")}:
                            <span className="font-medium text-foreground">
                              {" "}
                              {publishBehaviorLabel(selectedTask.execution_contract.publish_behavior)}
                            </span>
                          </div>
                          {selectedTask.execution_contract.artifact_path ? (
                            <div>
                              {bilingual("产物路径", "Artifact path")}:
                              <span className="font-medium text-foreground">
                                {" "}
                                {selectedTask.execution_contract.artifact_path}
                              </span>
                            </div>
                          ) : null}
                        </div>
                      </div>
                    </div>
                  </div>

              {selectedTask.delivery_tier === "session_scoped" ? (
                <div className="rounded-xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-800">
                  {bilingual(
                    "会话级任务会自动绑定到你当前的登录会话。当前登录会话失效、退出或被清理后，这个任务会自动停止并删除。",
                    "Session-scoped tasks automatically bind to your current login session. If that session expires, logs out, or is cleaned up, the task stops and is removed automatically.",
                  )}
                </div>
              ) : null}
              {selectedTask?.resume_hint ? (
                <div className="rounded-xl border border-blue-200 bg-blue-50 px-4 py-3 text-sm text-blue-800">
                  {selectedTask.resume_hint}
                </div>
              ) : null}
              {expiredOneShot ? (
                <div className="rounded-xl border border-blue-200 bg-blue-50 px-4 py-3 text-sm text-blue-800">
                  {bilingual(
                    "这是一条已经过期的一次性任务。它不能直接“恢复”到原来的时间点；如果还要再次执行，可以直接点“立即运行”，或者修改执行时间后重新启用。",
                    "This is an expired one-shot task. It cannot be restored to the original time point. If you still need it to run again, use “Run now” or modify the execution time and re-enable it.",
                  )}
                </div>
              ) : null}
              <div className="flex flex-wrap gap-2">
                {selectedTask ? (
                  <>
                    {selectedTask.status === "draft" || reenableCompletedOneShot ? (
                      <Button variant="outline" onClick={() => void runAction(() => scheduledTasksApi.publish(teamId, selectedTask.task_id))} disabled={saving}>
                        <Play className="mr-2 h-4 w-4" />
                        {reenableCompletedOneShot ? bilingual("重新启用", "Re-enable") : bilingual("发布启用", "Publish and enable")}
                      </Button>
                    ) : null}
                    {selectedTask.status === "active" ? (
                      <Button variant="outline" onClick={() => void runAction(() => scheduledTasksApi.pause(teamId, selectedTask.task_id))} disabled={saving}>
                        <Pause className="mr-2 h-4 w-4" />
                        {bilingual("暂停", "Pause")}
                      </Button>
                    ) : null}
                    {selectedTask.can_resume ? (
                      <Button variant="outline" onClick={() => void runAction(() => scheduledTasksApi.resume(teamId, selectedTask.task_id))} disabled={saving}>
                        <Play className="mr-2 h-4 w-4" />
                        {bilingual("恢复", "Resume")}
                      </Button>
                    ) : null}
                    <Button variant="outline" onClick={() => void runAction(() => scheduledTasksApi.runNow(teamId, selectedTask.task_id))} disabled={saving}>
                      <Clock3 className="mr-2 h-4 w-4" />
                      {bilingual("立即运行", "Run now")}
                    </Button>
                    {selectedTask.runs.some((run) => run.status === "running") ? (
                      <Button variant="outline" onClick={() => void runAction(() => scheduledTasksApi.cancel(teamId, selectedTask.task_id))} disabled={saving}>
                        {bilingual("取消运行", "Cancel run")}
                      </Button>
                    ) : null}
                    <Button variant="destructive" onClick={() => void runAction(() => scheduledTasksApi.remove(teamId, selectedTask.task_id))} disabled={saving}>
                      <Trash2 className="mr-2 h-4 w-4" />
                      {bilingual("删除", "Delete")}
                    </Button>
                  </>
                ) : null}
              </div>
              {selectedTask && canManage ? (
                <div className="border-t border-[hsl(var(--ui-line-soft))/0.72] pt-4">
                  <Button variant="outline" onClick={() => setEditOpen(true)} disabled={saving}>
                    {bilingual("编辑任务", "Edit task")}
                  </Button>
                </div>
              ) : null}
                </>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
        </TabsContent>

        <TabsContent value="activity" className="min-h-0 flex-1">
          {!selectedTask ? (
            <Card>
              <CardContent className="px-5 py-8 text-sm text-muted-foreground">
                {bilingual(
                  "先在“任务总览”里选择一个任务，这里会显示它的运行记录和频道触发历史。",
                  "Select a task in “Overview” first. This section then shows its run history and channel trigger history.",
                )}
              </CardContent>
            </Card>
          ) : (
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">{bilingual("最近运行", "Recent runs")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                {selectedTask.runs.map((run) => (
                  <div key={run.run_id} className="rounded-xl border px-4 py-3">
                    {(() => {
                      const errorDisplay = explainRunError(run.error, run.trigger_source, run.outcome_reason);
                      return (
                        <>
                    <div className="flex items-center justify-between gap-3">
                      <div className="text-sm font-medium">{triggerSourceLabel(run.trigger_source)}</div>
                      <div className={`text-sm font-medium ${taskStatusTone(run.status)}`}>{runStatusLabel(run.status)}</div>
                    </div>
                    <div className="mt-2 text-xs text-muted-foreground">
                      {bilingual("开始", "Started")}: {formatDateTimeDisplay(run.started_at)}
                      {run.finished_at ? ` · ${bilingual("结束", "Finished")}: ${formatDateTimeDisplay(run.finished_at)}` : ""}
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">
                      {bilingual("触发来源", "Trigger source")}: {triggerSourceLabel(run.trigger_source)}
                    </div>
                    {run.outcome_reason ? (
                      <div className="mt-1 text-xs text-muted-foreground">
                        {bilingual("结果判定", "Outcome")}: {runOutcomeReasonLabel(run.outcome_reason)}
                        {run.warning_count > 0 ? ` · ${bilingual("警告", "Warnings")} ${run.warning_count}` : ""}
                      </div>
                    ) : null}
                    {run.summary ? <div className="mt-2 text-sm text-foreground">{run.summary}</div> : null}
                    {run.self_evaluation ? (
                      <div className="mt-2 rounded-lg border border-blue-200 bg-blue-50 px-3 py-2 text-sm text-blue-900">
                        <div className="font-medium">
                          {bilingual("AI 自评", "AI self-evaluation")}: {run.self_evaluation.score} {bilingual("分", "")}
                          {selfEvaluationGradeLabel(run.self_evaluation.grade)
                            ? ` · ${selfEvaluationGradeLabel(run.self_evaluation.grade)}`
                            : ""}
                          {Number.isFinite(run.self_evaluation.confidence)
                            ? ` · ${bilingual("置信度", "Confidence")} ${Math.round(run.self_evaluation.confidence * 100)}%`
                            : ""}
                        </div>
                        {run.improvement_loop_applied && run.initial_self_evaluation ? (
                          <div className="mt-1 rounded-md border border-emerald-200 bg-emerald-50 px-2.5 py-2 text-xs text-emerald-900">
                            <div className="font-medium">
                              Improvement loop applied
                              {run.improvement_loop_count > 0 ? ` · ${run.improvement_loop_count} pass` : ""}
                            </div>
                            <div className="mt-1 text-emerald-800">
                              Initial score {run.initial_self_evaluation.score}
                              {selfEvaluationGradeLabel(run.initial_self_evaluation.grade)
                                ? ` · ${selfEvaluationGradeLabel(run.initial_self_evaluation.grade)}`
                                : ""}
                              {" → "}
                              Final score {run.self_evaluation.score}
                              {selfEvaluationGradeLabel(run.self_evaluation.grade)
                                ? ` · ${selfEvaluationGradeLabel(run.self_evaluation.grade)}`
                                : ""}
                            </div>
                            <div className="mt-1 text-emerald-800">
                              The system used the weak areas from the initial evaluation to run one focused improvement pass and then rescored the final result.
                            </div>
                          </div>
                        ) : null}
                        <div className="mt-1 text-blue-800">{run.self_evaluation.summary}</div>
                        {(run.self_evaluation.completed_steps?.length ||
                          run.self_evaluation.failed_steps?.length ||
                          run.self_evaluation.risks?.length) ? (
                          <details className="mt-2 text-xs text-blue-900/80">
                            <summary className="cursor-pointer select-none font-medium">
                              {bilingual("查看自评细节", "View self-evaluation details")}
                            </summary>
                            {run.self_evaluation.completed_steps?.length ? (
                              <div className="mt-2">
                                <div className="font-medium text-blue-900">{bilingual("已完成", "Completed")}</div>
                                <ul className="mt-1 list-disc pl-5 space-y-1">
                                  {run.self_evaluation.completed_steps.map((item) => (
                                    <li key={`done-${run.run_id}-${item}`}>{item}</li>
                                  ))}
                                </ul>
                              </div>
                            ) : null}
                            {run.self_evaluation.failed_steps?.length ? (
                              <div className="mt-2">
                                <div className="font-medium text-blue-900">{bilingual("未完成 / 失败", "Incomplete / failed")}</div>
                                <ul className="mt-1 list-disc pl-5 space-y-1">
                                  {run.self_evaluation.failed_steps.map((item) => (
                                    <li key={`fail-${run.run_id}-${item}`}>{item}</li>
                                  ))}
                                </ul>
                              </div>
                            ) : null}
                            {run.self_evaluation.risks?.length ? (
                              <div className="mt-2">
                                <div className="font-medium text-blue-900">{bilingual("风险", "Risks")}</div>
                                <ul className="mt-1 list-disc pl-5 space-y-1">
                                  {run.self_evaluation.risks.map((item) => (
                                    <li key={`risk-${run.run_id}-${item}`}>{item}</li>
                                  ))}
                                </ul>
                              </div>
                            ) : null}
                          </details>
                        ) : null}
                      </div>
                    ) : null}
                    {isSelfEvaluationPending(run) ? (
                      <div className="mt-2 rounded-lg border border-sky-200 bg-sky-50 px-3 py-2 text-sm text-sky-900">
                        <div className="font-medium">{bilingual("AI 自评生成中", "AI self-evaluation in progress")}</div>
                        <div className="mt-1 text-sky-800">
                          {bilingual(
                            "任务结果已经完成，评分和质量说明会在后台异步回填。",
                            "The task result is complete. Scoring and quality notes will be filled in asynchronously in the background.",
                          )}
                        </div>
                      </div>
                    ) : null}
                    {errorDisplay ? (
                      <div className="mt-2 rounded-lg border border-red-200 bg-red-50 px-3 py-2">
                        <div className="text-sm font-medium text-red-700">{errorDisplay.title}</div>
                        <div className="mt-1 text-sm text-red-600">{errorDisplay.detail}</div>
                      </div>
                    ) : null}
                    {run.error ? (
                      <details className="mt-2 text-xs text-muted-foreground">
                        <summary className="cursor-pointer select-none">{bilingual("查看技术细节", "View technical details")}</summary>
                        <div className="mt-1 whitespace-pre-wrap break-all">{run.error}</div>
                      </details>
                    ) : null}
                        </>
                      );
                    })()}
                  </div>
                ))}
                {!selectedTask.runs.length ? (
                  <div className="rounded-xl border border-dashed px-4 py-6 text-sm text-muted-foreground">
                    {bilingual("这个任务还没有运行记录。", "This task does not have any run history yet.")}
                  </div>
                ) : null}
              </CardContent>
            </Card>
          )}

          {selectedTask ? (
            <Card>
              <CardHeader className="pb-3">
                <CardTitle className="text-base">{bilingual("频道触发历史", "Channel trigger history")}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                {messages.map((message) => (
                  <div key={message.message_id} className="rounded-xl border px-4 py-3">
                    <div className="flex items-center justify-between gap-3">
                      <div className="inline-flex items-center gap-2 text-sm font-medium">
                        <Bot className="h-4 w-4 text-muted-foreground" />
                        {message.author_name}
                      </div>
                      <div className="text-xs text-muted-foreground">{message.created_at}</div>
                    </div>
                    <div className="mt-2 whitespace-pre-wrap text-sm text-foreground">{message.content_text}</div>
                  </div>
                ))}
                {!messages.length ? (
                  <div className="rounded-xl border border-dashed px-4 py-6 text-sm text-muted-foreground">
                    {bilingual("这个任务频道还没有触发消息。", "This task channel does not have any trigger messages yet.")}
                  </div>
                ) : null}
              </CardContent>
            </Card>
          ) : null}
        </TabsContent>
      </Tabs>

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent className="sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>{bilingual("编辑任务", "Edit task")}</DialogTitle>
            <DialogDescription>
              {bilingual(
                "编辑能力保留在独立弹层里，主页面继续保持只读摘要，避免重新回到旧的单页大表单。",
                "Editing stays in a separate dialog while the main page remains a read-only summary, so you do not fall back into the old monolithic form.",
              )}
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <label className="text-sm font-medium">{bilingual("标题", "Title")}</label>
              <Input value={form.title} onChange={(event) => setForm((prev) => ({ ...prev, title: event.target.value }))} />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">{bilingual("执行 Agent", "Execution agent")}</label>
              <Select value={form.agentId} onValueChange={(value) => setForm((prev) => ({ ...prev, agentId: value }))}>
                <SelectTrigger>
                  <SelectValue placeholder={bilingual("选择 Agent", "Select agent")} />
                </SelectTrigger>
                <SelectContent>
                  {agents.map((agent) => (
                    <SelectItem key={agent.id} value={agent.id}>
                      {agent.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">{bilingual("任务类型", "Task type")}</label>
              <Select value={form.taskKind} onValueChange={(value) => setForm((prev) => ({ ...prev, taskKind: value as ScheduledTaskKind }))}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="one_shot">{bilingual("一次性任务", "One-shot task")}</SelectItem>
                  <SelectItem value="cron">{bilingual("周期任务", "Recurring task")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">{bilingual("调度层级", "Delivery tier")}</label>
              <Select value={form.deliveryTier} onValueChange={(value) => setForm((prev) => ({ ...prev, deliveryTier: value as ScheduledTaskDeliveryTier }))}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="durable">{bilingual("持久化任务", "Durable task")}</SelectItem>
                  <SelectItem value="session_scoped">{bilingual("会话级任务", "Session-scoped task")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2 md:col-span-2">
              <label className="text-sm font-medium">{bilingual("执行时区", "Execution time zone")}</label>
              <Input
                list="scheduled-task-edit-timezone-options"
                value={form.timezone}
                onChange={(event) => setForm((prev) => ({ ...prev, timezone: event.target.value }))}
                placeholder="Asia/Shanghai"
              />
              <datalist id="scheduled-task-edit-timezone-options">
                {timezoneOptions.map((timezone) => (
                  <option key={timezone} value={timezone} />
                ))}
              </datalist>
            </div>
            {form.taskKind === "one_shot" ? (
              <div className="space-y-2 md:col-span-2">
              <label className="text-sm font-medium">{bilingual("执行时间", "Execution time")}</label>
                <Input type="datetime-local" value={form.oneShotAt} onChange={(event) => setForm((prev) => ({ ...prev, oneShotAt: event.target.value }))} />
                <div className="text-xs text-muted-foreground">{scheduleSummaryFromForm(form)}</div>
              </div>
            ) : (
              <div className="space-y-3 md:col-span-2">
                <label className="text-sm font-medium">{bilingual("重复频率", "Recurrence")}</label>
                <Select value={form.cronMode} onValueChange={(value) => setForm((prev) => ({ ...prev, cronMode: value as CronMode }))}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="every_minutes">{bilingual("每隔一段分钟", "Every N minutes")}</SelectItem>
                    <SelectItem value="every_hours">{bilingual("每隔几小时整点", "Every N hours on the hour")}</SelectItem>
                    <SelectItem value="daily_at">{bilingual("每天固定时间", "Daily at a fixed time")}</SelectItem>
                    <SelectItem value="weekdays_at">{bilingual("每个工作日固定时间", "Weekdays at a fixed time")}</SelectItem>
                    <SelectItem value="weekly_on">{bilingual("每周指定星期几", "Weekly on selected weekdays")}</SelectItem>
                    <SelectItem value="custom">{bilingual("高级：自定义 Cron 表达式", "Advanced: custom cron expression")}</SelectItem>
                  </SelectContent>
                </Select>
                {form.cronMode === "every_minutes" ? (
                  <Input type="number" min="1" step="1" value={form.everyMinutes} onChange={(event) => setForm((prev) => ({ ...prev, everyMinutes: event.target.value }))} />
                ) : null}
                {form.cronMode === "every_hours" ? (
                  <Input type="number" min="1" step="1" value={form.everyHours} onChange={(event) => setForm((prev) => ({ ...prev, everyHours: event.target.value }))} />
                ) : null}
                {form.cronMode === "daily_at" || form.cronMode === "weekdays_at" ? (
                  <Input type="time" value={normalizeDailyTime(form.dailyTime)} onChange={(event) => setForm((prev) => ({ ...prev, dailyTime: event.target.value }))} />
                ) : null}
                {form.cronMode === "weekly_on" ? (
                  <div className="space-y-3">
                    <div className="flex flex-wrap gap-2">
                      {WEEKDAY_OPTIONS.map((item) => {
                        const selected = form.weeklyDays.includes(item.value);
                        return (
                          <button
                            key={item.value}
                            type="button"
                            className={`rounded-full border px-3 py-1 text-xs transition ${
                              selected
                                ? "border-primary bg-primary/10 text-foreground"
                                : "border-[hsl(var(--ui-line-soft))/0.72] text-muted-foreground hover:bg-muted/40"
                            }`}
                            onClick={() =>
                              setForm((prev) => {
                                const next = selected
                                  ? prev.weeklyDays.filter((day) => day !== item.value)
                                  : [...prev.weeklyDays, item.value];
                                return { ...prev, weeklyDays: normalizeWeeklyDays(next) };
                              })
                            }
                          >
                            {bilingual(item.zh, item.en)}
                          </button>
                        );
                      })}
                    </div>
                    <Input type="time" value={normalizeDailyTime(form.dailyTime)} onChange={(event) => setForm((prev) => ({ ...prev, dailyTime: event.target.value }))} />
                  </div>
                ) : null}
                {form.cronMode === "custom" ? (
                  <Input value={form.cronExpression} onChange={(event) => setForm((prev) => ({ ...prev, cronExpression: event.target.value }))} placeholder="*/15 * * * *" />
                ) : null}
                <div className="text-xs text-muted-foreground">{scheduleSummaryFromForm(form)}</div>
              </div>
            )}
            <div className="space-y-2 md:col-span-2">
              <label className="text-sm font-medium">Prompt</label>
              <Textarea value={form.prompt} onChange={(event) => setForm((prev) => ({ ...prev, prompt: event.target.value }))} rows={8} />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditOpen(false)} disabled={saving}>
              {bilingual("取消", "Cancel")}
            </Button>
            <Button onClick={() => void handleUpdate()} disabled={saving || !selectedTaskId}>
              {saving ? bilingual("保存中...", "Saving...") : bilingual("保存修改", "Save changes")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
