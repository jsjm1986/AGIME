import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BottomSheetPanel } from "../mobile/BottomSheetPanel";
import type {
  ComposerCapabilitiesCatalog,
  ComposerCapabilityExtension,
  ComposerCapabilitySkill,
  ComposerHiddenCapabilityExtension,
} from "../../api/chat";

export interface ChatCapabilitySelection {
  key: string;
  kind: "skill" | "extension";
  name: string;
  ref: string;
  displayLineZh: string;
  plainLineZh: string;
  description?: string | null;
  summaryText?: string | null;
  detailText?: string | null;
  detailLang?: string | null;
  detailSource?: string | null;
  badge?: string | null;
}

interface ChatCapabilityPickerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  catalog: ComposerCapabilitiesCatalog | null;
  loading: boolean;
  error?: string | null;
  initialSelectedKeys?: string[];
  initialDetailKey?: string | null;
  onInsert: (items: ChatCapabilitySelection[]) => void;
}

type PickerTab = "skills" | "extensions";

interface PickerItem {
  key: string;
  kind: "skill" | "extension";
  title: string;
  description?: string | null;
  summaryText?: string | null;
  detailText?: string | null;
  detailLang?: string | null;
  detailSource?: string | null;
  badge?: string | null;
  ref: string;
  displayLineZh: string;
  plainLineZh: string;
}

function mapSkillItem(skill: ComposerCapabilitySkill): PickerItem {
  return {
    key: `skill:${skill.id}`,
    kind: "skill",
    title: skill.name,
    description: skill.description,
    summaryText: skill.summary_text,
    detailText: skill.detail_text,
    detailLang: skill.detail_lang,
    detailSource: skill.detail_source,
    badge: skill.version ? `v${skill.version}` : null,
    ref: skill.skill_ref,
    displayLineZh: skill.display_line_zh,
    plainLineZh: skill.plain_line_zh,
  };
}

function mapExtensionItem(extension: ComposerCapabilityExtension): PickerItem {
  const typeLabel = extension.type
    ? extension.type === "streamable_http"
      ? "HTTP"
      : extension.type.toUpperCase()
    : extension.class === "builtin"
      ? "内置"
      : extension.class === "team"
        ? "团队"
        : "扩展";
  return {
    key: `ext:${extension.runtime_name}`,
    kind: "extension",
    title: extension.display_name,
    description: extension.description,
    summaryText: extension.summary_text,
    detailText: extension.detail_text,
    detailLang: extension.detail_lang,
    detailSource: extension.detail_source,
    badge: typeLabel,
    ref: extension.ext_ref,
    displayLineZh: extension.display_line_zh,
    plainLineZh: extension.plain_line_zh,
  };
}

function describeHiddenExtension(
  item: ComposerHiddenCapabilityExtension,
  t: (key: string, fallback: string) => string,
): string {
  switch (item.reason) {
    case "skill_runtime":
      return t(
        "chat.capabilityPickerHiddenReason.skillRuntime",
        "这是技能运行时入口，已归入“技能”标签，不在 MCP / 扩展列表里重复显示。",
      );
    case "system_assist":
      return t(
        "chat.capabilityPickerHiddenReason.systemAssist",
        "这是系统辅助扩展，不提供聊天里的显式手动调用入口。",
      );
    case "legacy_hidden":
      return t(
        "chat.capabilityPickerHiddenReason.legacyHidden",
        "这是遗留扩展，当前 team server 运行时不会暴露给聊天会话。",
      );
    default:
      return t(
        "chat.capabilityPickerHiddenReason.default",
        "这个扩展当前不会出现在聊天显式调用列表中。",
      );
  }
}

function buildSelection(item: PickerItem): ChatCapabilitySelection {
  return {
    key: item.key,
    kind: item.kind,
    name: item.title,
    ref: item.ref,
    displayLineZh: item.displayLineZh,
    plainLineZh: item.plainLineZh,
    description: item.description,
    summaryText: item.summaryText,
    detailText: item.detailText,
    detailLang: item.detailLang,
    detailSource: item.detailSource,
    badge: item.badge,
  };
}

function isChineseLang(value?: string | null): boolean {
  return (value || "").trim().toLowerCase().startsWith("zh");
}

export function ChatCapabilityPicker({
  open,
  onOpenChange,
  catalog,
  loading,
  error,
  initialSelectedKeys = [],
  initialDetailKey = null,
  onInsert,
}: ChatCapabilityPickerProps) {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<PickerTab>("skills");
  const [query, setQuery] = useState("");
  const [selectedKeys, setSelectedKeys] = useState<string[]>(initialSelectedKeys);
  const [detailKey, setDetailKey] = useState<string | null>(null);
  const listScrollRef = useRef<HTMLDivElement | null>(null);
  const listScrollTopRef = useRef<Record<PickerTab, number>>({
    skills: 0,
    extensions: 0,
  });

  const itemsByTab = useMemo(
    () => ({
      skills: (catalog?.skills || []).map(mapSkillItem),
      extensions: (catalog?.extensions || []).map(mapExtensionItem),
    }),
    [catalog],
  );

  const itemsByKey = useMemo(() => {
    const entries = new Map<string, PickerItem>();
    [...itemsByTab.skills, ...itemsByTab.extensions].forEach((item) => {
      entries.set(item.key, item);
    });
    return entries;
  }, [itemsByTab.extensions, itemsByTab.skills]);

  useEffect(() => {
    if (!open) {
      return;
    }
    setSelectedKeys(initialSelectedKeys);
    setQuery("");
    const detailItem = initialDetailKey ? itemsByKey.get(initialDetailKey) : null;
    if (detailItem) {
      setActiveTab(detailItem.kind === "skill" ? "skills" : "extensions");
      setDetailKey(detailItem.key);
    } else if (itemsByTab.skills.length === 0 && itemsByTab.extensions.length > 0) {
      setActiveTab("extensions");
      setDetailKey(null);
    } else {
      setActiveTab("skills");
      setDetailKey(null);
    }
  }, [
    initialDetailKey,
    initialSelectedKeys,
    itemsByKey,
    itemsByTab.extensions.length,
    itemsByTab.skills.length,
    open,
  ]);

  const filteredItems = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    const items = activeTab === "skills" ? itemsByTab.skills : itemsByTab.extensions;
    if (!normalizedQuery) {
      return items;
    }
    return items.filter((item) => {
      const haystack = [
        item.title,
        item.summaryText || "",
        item.detailText || "",
        item.description || "",
        item.displayLineZh,
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(normalizedQuery);
    });
  }, [activeTab, itemsByTab.extensions, itemsByTab.skills, query]);

  const hiddenExtensions = catalog?.hidden_extensions || [];
  const extensionVisibleCount = itemsByTab.extensions.length;
  const extensionEnabledTotal = extensionVisibleCount + hiddenExtensions.length;

  const selectedItems = useMemo(() => {
    const selected = new Map<string, PickerItem>();
    itemsByKey.forEach((item, key) => {
      if (selectedKeys.includes(key)) {
        selected.set(key, item);
      }
    });
    return selectedKeys
      .map((key) => selected.get(key))
      .filter((item): item is PickerItem => Boolean(item));
  }, [itemsByKey, selectedKeys]);

  const detailItem = detailKey ? itemsByKey.get(detailKey) || null : null;
  const detailHasRawDescription =
    !!detailItem?.description &&
    detailItem.description !== detailItem.detailText &&
    detailItem.detailSource !== "raw_description";
  const detailUsesRawDescription = detailItem?.detailSource === "raw_description";
  const detailShowsChinese = isChineseLang(detailItem?.detailLang);

  const toggleItem = (key: string) => {
    setSelectedKeys((prev) =>
      prev.includes(key) ? prev.filter((item) => item !== key) : [...prev, key],
    );
  };

  const openDetail = (itemKey: string) => {
    if (listScrollRef.current) {
      listScrollTopRef.current[activeTab] = listScrollRef.current.scrollTop;
    }
    setDetailKey(itemKey);
  };

  const closeDetail = () => {
    const restoreTab = activeTab;
    setDetailKey(null);
    requestAnimationFrame(() => {
      if (listScrollRef.current) {
        listScrollRef.current.scrollTop = listScrollTopRef.current[restoreTab] || 0;
      }
    });
  };

  const handleInsert = () => {
    onInsert(selectedItems.map(buildSelection));
  };

  return (
    <BottomSheetPanel
      open={open}
      onOpenChange={onOpenChange}
      title={
        detailItem
          ? t("chat.capabilityPickerDetailTitle", "能力解读")
          : t("chat.capabilityPickerTitle", "选择技能与扩展")
      }
      description={
        detailItem
          ? undefined
          : t(
              "chat.capabilityPickerHint",
              "只显示当前 Agent / 当前会话真正可调用的技能和 MCP 扩展。",
            )
      }
      fullHeight
    >
      <div className="flex min-h-[460px] flex-col gap-3">
        {detailItem ? (
          <>
            <div className="flex items-start gap-3 border-b border-border/60 pb-3">
              <button
                type="button"
                onClick={closeDetail}
                className="inline-flex h-9 items-center justify-center rounded-[12px] border border-border/70 px-3 text-[12px] font-medium text-foreground transition-colors hover:bg-muted/35"
              >
                {t("common.back", "返回")}
              </button>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-[15px] font-semibold text-foreground">
                    {detailItem.title}
                  </h3>
                  {detailItem.badge && (
                    <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
                      {detailItem.badge}
                    </span>
                  )}
                  {detailItem.detailSource && (
                    <span className="rounded-full border border-border/70 bg-background px-2 py-0.5 text-[10px] text-muted-foreground">
                      {detailItem.detailSource === "raw_description"
                        ? t("chat.capabilityPickerDetailSourceRaw", "原始说明")
                        : t("chat.capabilityPickerDetailSourceAi", "AI 解读")}
                    </span>
                  )}
                </div>
                {detailItem.detailLang && (
                  <div className="mt-1 text-[11px] text-muted-foreground">
                    {t("chat.capabilityPickerDetailLang", "语言：{{lang}}", {
                      lang: detailItem.detailLang,
                    })}
                  </div>
                )}
              </div>
            </div>

            <div className="min-h-0 flex-1 overflow-y-auto pr-1">
              <div className="rounded-[20px] border border-border/60 bg-card/98 px-4 py-4">
                {detailUsesRawDescription && (
                  <div className="mb-4 rounded-[16px] border border-warning/20 bg-warning/8 px-3 py-2 text-[12px] leading-5 text-foreground">
                    {detailShowsChinese
                      ? t(
                          "chat.capabilityPickerRawDescriptionZh",
                          "当前展示的是这项能力的原始说明。",
                        )
                      : t(
                          "chat.capabilityPickerRawDescriptionNoZh",
                          "当前没有可用的中文解读，正在展示这项能力的原始说明。",
                        )}
                  </div>
                )}
                <div className="whitespace-pre-wrap text-[13px] leading-6 text-foreground">
                  {detailItem.detailText ||
                    detailItem.description ||
                    detailItem.plainLineZh ||
                    t(
                      "chat.capabilityPickerNoDetail",
                      "当前没有可展示的能力解读，可以直接选择后插入到输入框。",
                    )}
                </div>

                {detailHasRawDescription && (
                  <details className="mt-4 rounded-[16px] border border-border/60 bg-muted/18 px-3 py-2">
                    <summary className="cursor-pointer text-[12px] font-medium text-foreground">
                      {t("chat.capabilityPickerRawDescription", "查看原始说明")}
                    </summary>
                    <div className="mt-2 whitespace-pre-wrap text-[12px] leading-5 text-muted-foreground">
                      {detailItem.description}
                    </div>
                  </details>
                )}
              </div>
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-border/60 pt-3">
              <div className="text-[12px] text-muted-foreground">
                {selectedKeys.includes(detailItem.key)
                  ? t("chat.capabilityPickerAlreadySelected", "已加入本轮调用")
                  : t("chat.capabilityPickerReadyToSelect", "确认后会插入到输入框")}
              </div>
              <button
                type="button"
                onClick={() => {
                  if (!selectedKeys.includes(detailItem.key)) {
                    setSelectedKeys((prev) => [...prev, detailItem.key]);
                  }
                  closeDetail();
                }}
                className={`inline-flex h-10 items-center justify-center rounded-[14px] px-4 text-[12px] font-medium transition-colors ${
                  selectedKeys.includes(detailItem.key)
                    ? "border border-primary/25 bg-primary/10 text-primary hover:bg-primary/14"
                    : "bg-primary text-primary-foreground hover:opacity-95"
                }`}
              >
                {selectedKeys.includes(detailItem.key)
                  ? t("chat.capabilityPickerAdded", "已加入本轮调用")
                  : t("chat.capabilityPickerSelectThis", "选择此能力")}
              </button>
            </div>
          </>
        ) : (
          <>
            <div className="flex gap-2 rounded-[18px] border border-border/60 bg-muted/18 p-1">
              {([
                ["skills", t("chat.capabilityPickerSkills", "技能"), itemsByTab.skills.length],
                [
                  "extensions",
                  t("chat.capabilityPickerExtensions", "MCP / 扩展"),
                  hiddenExtensions.length > 0
                    ? `${extensionVisibleCount} / ${extensionEnabledTotal}`
                    : extensionVisibleCount,
                ],
              ] as const).map(([tab, label, count]) => {
                const active = activeTab === tab;
                return (
                  <button
                    key={tab}
                    type="button"
                    onClick={() => setActiveTab(tab)}
                    className={`flex-1 rounded-[14px] px-3 py-2 text-[12px] font-medium transition-colors ${
                      active
                        ? "bg-primary text-primary-foreground shadow-sm"
                        : "text-muted-foreground hover:bg-background/80 hover:text-foreground"
                    }`}
                  >
                    {label}{" "}
                    <span
                      className={`text-[11px] ${
                        active ? "text-primary-foreground/80" : "text-muted-foreground"
                      }`}
                    >
                      {count}
                    </span>
                  </button>
                );
              })}
            </div>

            <input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("chat.capabilityPickerSearch", "搜索名称、说明或能力用途")}
              className="h-11 rounded-[16px] border border-border/70 bg-background px-4 text-[13px] outline-none focus:ring-2 focus:ring-ring"
            />

            {selectedItems.length > 0 && (
              <div className="flex flex-wrap gap-1.5">
                {selectedItems.map((item) => (
                  <span
                    key={item.key}
                    className="inline-flex items-center gap-1 rounded-full border border-primary/20 bg-primary/8 px-2 py-1 text-[11px] font-medium text-primary"
                  >
                    <button
                      type="button"
                      onClick={() => openDetail(item.key)}
                      className="truncate max-w-[180px] text-left"
                    >
                      {item.title}
                    </button>
                    <button
                      type="button"
                      onClick={() => toggleItem(item.key)}
                      className="text-primary/70 transition-colors hover:text-primary"
                      title={t("chat.removeCapability", "移除该能力引用")}
                    >
                      ×
                    </button>
                  </span>
                ))}
              </div>
            )}

            {activeTab === "extensions" && hiddenExtensions.length > 0 && (
              <div className="rounded-[18px] border border-border/60 bg-muted/18 px-4 py-3 text-[12px] text-muted-foreground">
                <div className="font-medium text-foreground">
                  {t(
                    "chat.capabilityPickerExtensionsScopeTitle",
                    "当前会话可显式调用 {{visible}} 个 / Agent 已启用 {{total}} 个",
                    {
                      visible: extensionVisibleCount,
                      total: extensionEnabledTotal,
                    },
                  )}
                </div>
                <div className="mt-1 leading-5">
                  {t(
                    "chat.capabilityPickerExtensionsScopeBody",
                    "这里显示的是当前会话可手动点选调用的 MCP / 扩展。其余已启用项没有丢失，只是按角色被归到技能或系统辅助层。",
                  )}
                </div>
                <div className="mt-2 space-y-1.5">
                  {hiddenExtensions.map((item) => (
                    <div
                      key={item.runtime_name}
                      className="rounded-[14px] border border-border/60 bg-background/88 px-3 py-2"
                    >
                      <div className="text-[12px] font-medium text-foreground">
                        {item.display_name}
                      </div>
                      <div className="mt-0.5 text-[11px] leading-5 text-muted-foreground">
                        {describeHiddenExtension(item, t)}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            <div ref={listScrollRef} className="min-h-0 flex-1 overflow-y-auto pr-1">
              {loading ? (
                <div className="rounded-[18px] border border-border/60 bg-muted/16 px-4 py-6 text-[12px] text-muted-foreground">
                  {t("common.loading", "加载中...")}
                </div>
              ) : error ? (
                <div className="rounded-[18px] border border-status-error-text/20 bg-status-error-bg px-4 py-6 text-[12px] text-status-error-text">
                  {error}
                </div>
              ) : filteredItems.length === 0 ? (
                <div className="rounded-[18px] border border-border/60 bg-muted/16 px-4 py-6 text-[12px] text-muted-foreground">
                  {t("chat.capabilityPickerEmpty", "当前没有可选的技能或扩展。")}
                </div>
              ) : (
                <div className="space-y-2">
                  {filteredItems.map((item) => {
                    const active = selectedKeys.includes(item.key);
                    return (
                      <div
                        key={item.key}
                        className={`rounded-[18px] border px-4 py-3 transition-colors ${
                          active
                            ? "border-primary/35 bg-primary/8"
                            : "border-border/70 bg-card/96"
                        }`}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-2">
                              <div className="truncate text-[13px] font-medium text-foreground">
                                {item.title}
                              </div>
                              {item.badge && (
                                <span className="rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
                                  {item.badge}
                                </span>
                              )}
                              {active && (
                                <span className="rounded-full border border-primary/20 bg-primary/8 px-2 py-0.5 text-[10px] text-primary">
                                  {t("chat.capabilityPickerSelected", "已选择")}
                                </span>
                              )}
                            </div>
                            <div className="mt-1 line-clamp-2 text-[11px] leading-5 text-muted-foreground">
                              {item.summaryText || item.description || item.plainLineZh}
                            </div>
                          </div>
                        </div>
                        <div className="mt-3 flex items-center justify-between gap-2">
                          <button
                            type="button"
                            onClick={() => openDetail(item.key)}
                            className="inline-flex h-9 items-center justify-center rounded-[12px] border border-border/70 px-3 text-[11px] font-medium text-foreground transition-colors hover:bg-muted/35"
                          >
                            {t("chat.capabilityPickerViewDetail", "查看解读")}
                          </button>
                          <button
                            type="button"
                            onClick={() => toggleItem(item.key)}
                            className={`inline-flex h-9 items-center justify-center rounded-[12px] px-3 text-[11px] font-medium transition-colors ${
                              active
                                ? "border border-primary/25 bg-primary/10 text-primary hover:bg-primary/14"
                                : "bg-primary text-primary-foreground hover:opacity-95"
                            }`}
                          >
                            {active
                              ? t("chat.capabilityPickerChosen", "已选择")
                              : t("chat.capabilityPickerChoose", "选择")}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>

            <div className="flex items-center justify-between gap-3 border-t border-border/60 pt-3">
              <span className="text-[12px] text-muted-foreground">
                {t("chat.capabilityPickerSelectedCount", "已选 {{count}} 项", {
                  count: selectedItems.length,
                })}
              </span>
              <button
                type="button"
                onClick={handleInsert}
                disabled={selectedItems.length === 0}
                className="inline-flex h-10 items-center justify-center rounded-[14px] bg-primary px-4 text-[12px] font-medium text-primary-foreground transition-opacity disabled:cursor-not-allowed disabled:opacity-50"
              >
                {t("chat.capabilityPickerInsert", "插入到输入框")}
              </button>
            </div>
          </>
        )}
      </div>
    </BottomSheetPanel>
  );
}
