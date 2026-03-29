import {
  useCallback,
  useEffect,
  useState,
} from "react";
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
      setError("当前无法读取个人记忆，请稍后再试。");
    } finally {
      setLoading(false);
    }
  }, [syncDraft, teamId]);

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
    ? `仅在「${teamName}」团队的普通对话里生效`
    : "仅在当前团队的普通对话里生效";

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
      setError("保存失败，请稍后再试。");
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
    teamId,
  ]);

  const editorBody = (
    <div className="space-y-4">
      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2 text-[11px] font-medium text-muted-foreground">
          <span className="inline-flex items-center rounded-full border border-border/70 bg-muted/30 px-2.5 py-1">
            个人记忆
          </span>
          <span className="inline-flex items-center rounded-full border border-primary/15 bg-primary/[0.045] px-2.5 py-1 text-primary/80">
            {teamName ? `当前团队 · ${teamName}` : "当前团队"}
          </span>
        </div>
        <div className="space-y-1">
          <div className="text-[16px] font-semibold tracking-[-0.01em] text-foreground">
            和 {userDisplayName || "你"} 的熟悉感设置
          </div>
          <p className="text-[12px] leading-5 text-muted-foreground">
            这份记忆只作用于你在当前团队里的普通对话，不会共享给其他团队，也不会进入
            Agentify、portal 等功能型会话。
          </p>
        </div>
      </div>

      <div className="grid gap-4">
        <div className="space-y-1.5">
          <label className="text-sm font-medium">优先称呼</label>
          <Input
            value={draft.preferredAddress}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                preferredAddress: e.target.value,
              }))
            }
            placeholder="例如：agime / 老板 / 爸爸"
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">你希望我怎么和你说话</label>
          <Textarea
            value={draft.collaborationPreference}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                collaborationPreference: e.target.value,
              }))
            }
            className="min-h-[88px]"
            placeholder="例如：先给判断，再给建议；直接一点，少一点客服腔；需要时可以更像熟人交流。"
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">团队角色</label>
          <Input
            value={draft.roleHint}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                roleHint: e.target.value,
              }))
            }
            placeholder="例如：产品负责人 / 开发 / 运营"
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">最近关注</label>
          <Textarea
            value={draft.currentFocus}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                currentFocus: e.target.value,
              }))
            }
            className="min-h-[78px]"
            placeholder="例如：最近主要在打磨普通对话体验和 API 协同。"
          />
        </div>
        <div className="space-y-1.5">
          <label className="text-sm font-medium">备注</label>
          <Textarea
            value={draft.notes}
            onChange={(e) =>
              setDraft((prev) => ({
                ...prev,
                notes: e.target.value,
              }))
            }
            className="min-h-[88px]"
            placeholder="少量稳定信息即可，不要写成长档案。"
          />
        </div>
      </div>

      {error ? (
        <p className="text-sm text-status-error-text">{error}</p>
      ) : loading ? (
        <p className="text-sm text-muted-foreground">正在读取个人记忆…</p>
      ) : memoryRecord ? (
        <p className="text-xs text-muted-foreground">
          最近更新：{new Date(memoryRecord.updated_at).toLocaleString()}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          当前还没有保存任何个人记忆。
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
          取消
        </button>
        <button
          type="button"
          onClick={handleSave}
          disabled={saving}
          className="inline-flex h-9 items-center rounded-full bg-foreground px-4 text-[12px] font-medium text-background transition-colors hover:opacity-90 disabled:opacity-60"
        >
          {saving ? "保存中..." : "保存"}
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
          title={teamName ? `个人记忆 · ${teamName}` : "个人记忆"}
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
            "inline-flex h-5 items-center gap-1 rounded-full border border-transparent bg-transparent px-1 text-[9px] font-normal transition-colors",
            hasMemory
              ? "text-[hsl(var(--sidebar-foreground))] hover:text-[hsl(var(--sidebar-foreground))]"
              : "text-[hsl(var(--sidebar-foreground))/0.62] hover:text-[hsl(var(--sidebar-foreground))]",
          )}
          title={teamName ? `个人记忆 · ${teamName}` : "个人记忆"}
        >
          {loading ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3 w-3" />
          )}
          <span>个人记忆</span>
          {hasMemory ? (
            <span className="h-1.5 w-1.5 rounded-full bg-primary/80" />
          ) : null}
        </button>
      )}

      <BottomSheetPanel
        open={open}
        onOpenChange={setOpen}
        title="个人记忆"
        description={scopeLabel}
        fullHeight={!isMobile}
      >
        {editorBody}
      </BottomSheetPanel>
    </div>
  );
}
