import {
  useCallback,
  useEffect,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Loader2,
  Sparkles,
  UserRound,
} from "lucide-react";
import { chatApi, type UserChatMemory } from "../../api/chat";
import { useIsMobile } from "../../hooks/useMediaQuery";
import { cn } from "../../utils";
import { BottomSheetPanel } from "../mobile/BottomSheetPanel";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import {
  RELATIONSHIP_MEMORY_UPDATED_EVENT,
  dispatchRelationshipMemoryUpdated,
  type RelationshipMemoryPatchPayload,
  type RelationshipMemoryUpdatedDetail,
} from "./relationshipMemoryEvents";

type RelationshipMemoryControlVariant = "footer" | "icon";

interface RelationshipMemoryDraft {
  preferredAddress: string;
  roleHint: string;
  currentFocus: string;
  collaborationPreference: string;
  notes: string;
}

interface RelationshipMemoryControlProps {
  teamId: string;
  teamName?: string | null;
  userDisplayName?: string | null;
  variant?: RelationshipMemoryControlVariant;
  className?: string;
}

function createEmptyDraft(): RelationshipMemoryDraft {
  return {
    preferredAddress: "",
    roleHint: "",
    currentFocus: "",
    collaborationPreference: "",
    notes: "",
  };
}

function draftFromMemory(
  memory: UserChatMemory | null,
): RelationshipMemoryDraft {
  return {
    preferredAddress: memory?.preferred_address || "",
    roleHint: memory?.role_hint || "",
    currentFocus: memory?.current_focus || "",
    collaborationPreference: memory?.collaboration_preference || "",
    notes: memory?.notes || "",
  };
}

export function RelationshipMemoryControl({
  teamId,
  teamName,
  userDisplayName,
  variant = "footer",
  className,
}: RelationshipMemoryControlProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const isCompact = variant === "icon";
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [memoryRecord, setMemoryRecord] = useState<UserChatMemory | null>(null);
  const [draft, setDraft] = useState<RelationshipMemoryDraft>(createEmptyDraft);

  const syncDraft = useCallback((memory: UserChatMemory | null) => {
    setDraft(draftFromMemory(memory));
  }, []);

  const loadMemory = useCallback(async () => {
    if (!teamId) {
      setMemoryRecord(null);
      syncDraft(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const memory = await chatApi.getMyMemory(teamId);
      setMemoryRecord(memory);
      syncDraft(memory);
    } catch (loadError) {
      console.error("Failed to load relationship memory:", loadError);
      setError(
        t(
          "chat.relationshipMemory.loadFailed",
          "Personal memory is unavailable right now. Please try again later.",
        ),
      );
    } finally {
      setLoading(false);
    }
  }, [syncDraft, t, teamId]);

  useEffect(() => {
    void loadMemory();
  }, [loadMemory]);

  useEffect(() => {
    const handleUpdated = (rawEvent: Event) => {
      const event = rawEvent as CustomEvent<RelationshipMemoryUpdatedDetail>;
      if (event.detail?.teamId !== teamId) {
        return;
      }
      void loadMemory();
    };
    window.addEventListener(
      RELATIONSHIP_MEMORY_UPDATED_EVENT,
      handleUpdated as EventListener,
    );
    return () => {
      window.removeEventListener(
        RELATIONSHIP_MEMORY_UPDATED_EVENT,
        handleUpdated as EventListener,
      );
    };
  }, [loadMemory, teamId]);

  const scopeLabel = teamName
    ? t(
        "chat.relationshipMemory.scopeWithTeam",
        "Only applies to regular chats in the {{teamName}} team.",
        { teamName },
      )
    : t(
        "chat.relationshipMemory.scopeCurrentTeam",
        "Only applies to regular chats in the current team.",
      );

  const hasMemory =
    !!memoryRecord?.preferred_address ||
    !!memoryRecord?.role_hint ||
    !!memoryRecord?.current_focus ||
    !!memoryRecord?.collaboration_preference ||
    !!memoryRecord?.notes;

  const buildPayload = (): RelationshipMemoryPatchPayload => ({
    preferred_address: draft.preferredAddress.trim() || null,
    role_hint: draft.roleHint.trim() || null,
    current_focus: draft.currentFocus.trim() || null,
    collaboration_preference: draft.collaborationPreference.trim() || null,
    notes: draft.notes.trim() || null,
  });

  const handleSave = useCallback(async () => {
    setSaving(true);
    setError(null);
    const payload = buildPayload();
    try {
      const memory = await chatApi.updateMyMemory(teamId, payload);
      setMemoryRecord(memory);
      syncDraft(memory);
      setOpen(false);
      dispatchRelationshipMemoryUpdated({
        teamId,
        source: "sidebar",
        patch: payload,
      });
    } catch (saveError) {
      console.error("Failed to save relationship memory:", saveError);
      setError(t("chat.relationshipMemory.saveFailed", "Failed to save. Please try again later."));
    } finally {
      setSaving(false);
    }
  }, [
    draft.collaborationPreference,
    draft.currentFocus,
    draft.notes,
    draft.preferredAddress,
    draft.roleHint,
    syncDraft,
    t,
    teamId,
  ]);

  const editorBody = (
    <div className="space-y-4">
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2 text-[11px] font-medium text-muted-foreground">
          <span className="inline-flex items-center rounded-full border border-border/70 bg-muted/30 px-2.5 py-1">
            {t("chat.relationshipMemory.badge", "Personal memory")}
          </span>
          <span className="inline-flex items-center rounded-full border border-primary/15 bg-primary/[0.045] px-2.5 py-1 text-primary/80">
            {teamName
              ? t(
                  "chat.relationshipMemory.currentTeamWithName",
                  "Current team · {{teamName}}",
                  { teamName },
                )
              : t("chat.relationshipMemory.currentTeam", "Current team")}
          </span>
        </div>
        <div className="space-y-1">
          <div className="text-[16px] font-semibold tracking-[-0.01em] text-foreground">
            {t(
              "chat.relationshipMemory.heading",
              "How I should relate to {{userDisplayName}}",
              { userDisplayName: userDisplayName || t("chat.relationshipMemory.you", "you") },
            )}
          </div>
          <p className="text-[12px] leading-5 text-muted-foreground">
            {t(
              "chat.relationshipMemory.description",
              "This memory only applies to your regular chats in the current team. It is not shared with other teams and does not enter functional surfaces like Agentify or portal sessions.",
            )}
          </p>
        </div>
      </div>

      <div className="grid gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">
            {t("chat.relationshipMemory.preferredAddressLabel", "Preferred address")}
          </label>
          <Input
            value={draft.preferredAddress}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                preferredAddress: e.target.value,
              }))
            }
            placeholder={t(
              "chat.relationshipMemory.preferredAddressPlaceholder",
              "For example: agime / boss / dad",
            )}
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">
            {t(
              "chat.relationshipMemory.collaborationPreferenceLabel",
              "How should I talk to you",
            )}
          </label>
          <Textarea
            value={draft.collaborationPreference}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                collaborationPreference: e.target.value,
              }))
            }
            className="min-h-[88px]"
            placeholder={t(
              "chat.relationshipMemory.collaborationPreferencePlaceholder",
              "For example: give the judgment first, then suggestions; be direct; sound less like customer support; feel more familiar when appropriate.",
            )}
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">
            {t("chat.relationshipMemory.roleHintLabel", "Team role")}
          </label>
          <Input
            value={draft.roleHint}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                roleHint: e.target.value,
              }))
            }
            placeholder={t(
              "chat.relationshipMemory.roleHintPlaceholder",
              "For example: product lead / engineer / operations",
            )}
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">
            {t("chat.relationshipMemory.currentFocusLabel", "Current focus")}
          </label>
          <Textarea
            value={draft.currentFocus}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                currentFocus: e.target.value,
              }))
            }
            className="min-h-[78px]"
            placeholder={t(
              "chat.relationshipMemory.currentFocusPlaceholder",
              "For example: recently focused on improving regular chat UX and API collaboration.",
            )}
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">
            {t("chat.relationshipMemory.notesLabel", "Notes")}
          </label>
          <Textarea
            value={draft.notes}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                notes: e.target.value,
              }))
            }
            className="min-h-[88px]"
            placeholder={t(
              "chat.relationshipMemory.notesPlaceholder",
              "Keep it short and stable. Do not turn it into a long profile.",
            )}
          />
        </div>
      </div>

      {error ? (
        <p className="text-sm text-status-error-text">{error}</p>
      ) : loading ? (
        <p className="text-sm text-muted-foreground">
          {t("chat.relationshipMemory.loading", "Loading personal memory…")}
        </p>
      ) : memoryRecord ? (
        <p className="text-xs text-muted-foreground">
          {t("chat.relationshipMemory.lastUpdated", "最近更新：{{time}}", {
            time: new Date(memoryRecord.updated_at).toLocaleString(),
          })}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          {t(
            "chat.relationshipMemory.empty",
            "No personal memory has been saved yet.",
          )}
        </p>
      )}

      <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={() => {
            setOpen(false);
            syncDraft(memoryRecord);
            setError(null);
          }}
          className="inline-flex h-9 items-center rounded-full border border-border/70 bg-background px-4 text-[12px] font-medium text-muted-foreground transition-colors hover:bg-muted/40"
        >
          {t("common.cancel", "取消")}
        </button>
        <button
          type="button"
          onClick={handleSave}
          disabled={saving}
          className="inline-flex h-9 items-center rounded-full bg-foreground px-4 text-[12px] font-medium text-background transition-colors hover:opacity-90 disabled:opacity-60"
        >
          {saving
            ? t("common.saving", "保存中...")
            : t("common.save", "保存")}
        </button>
      </div>
    </div>
  );

  return (
    <div className={cn("relative", className)}>
      {isCompact ? (
        <button
          type="button"
          onClick={() => {
            setError(null);
            setOpen((prev) => !prev);
          }}
          className="flex h-8 w-8 items-center justify-center rounded-[12px] border border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))] text-[hsl(var(--sidebar-foreground))/0.82] transition-colors hover:border-[hsl(var(--sidebar-accent))/0.22] hover:bg-[hsl(var(--sidebar-accent))/0.08] hover:text-[hsl(var(--sidebar-foreground))]"
          title={
            teamName
              ? t(
                  "chat.relationshipMemory.titleWithTeam",
                  "Personal memory · {{teamName}}",
                  { teamName },
                )
              : t("chat.relationshipMemory.title", "Personal memory")
          }
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <UserRound className="h-3.5 w-3.5" />
          )}
        </button>
      ) : (
        <button
          type="button"
          onClick={() => {
            setError(null);
            setOpen((prev) => !prev);
          }}
          className={cn(
            "inline-flex h-5 items-center gap-1 px-0.5 text-[11px] font-semibold leading-4 transition-colors",
            hasMemory
              ? "text-[hsl(var(--sidebar-foreground))/0.84] hover:text-[hsl(var(--sidebar-foreground))]"
              : "text-[hsl(var(--sidebar-foreground))/0.6] hover:text-[hsl(var(--sidebar-foreground))]",
          )}
          title={
            teamName
              ? t(
                  "chat.relationshipMemory.titleWithTeam",
                  "Personal memory · {{teamName}}",
                  { teamName },
                )
              : t("chat.relationshipMemory.title", "Personal memory")
          }
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" />
          )}
          <span>{t("chat.relationshipMemory.title", "Personal memory")}</span>
          {hasMemory ? (
            <span className="h-1.5 w-1.5 rounded-full bg-primary/80" />
          ) : null}
        </button>
      )}

      <BottomSheetPanel
        open={open}
        onOpenChange={setOpen}
        title={t("chat.relationshipMemory.title", "Personal memory")}
        description={scopeLabel}
        fullHeight={!isMobile}
      >
        {editorBody}
      </BottomSheetPanel>
    </div>
  );
}
