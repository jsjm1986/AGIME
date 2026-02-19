import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { IoIosCloseCircle, IoIosInformationCircle, IoIosWarning } from 'react-icons/io';
import { FaPencilAlt, FaSave } from 'react-icons/fa';
import { cn } from '../../utils';
import { Alert, AlertType } from './types';
import { upsertConfig } from '../../api';
import { useConfig } from '../ConfigContext';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '../ui/dialog';
import {
  createMemoryFact,
  listMemoryFacts,
  listMemoryCandidates,
  listMemorySnapshots,
  listMemoryToolGateEvents,
  MemoryCandidate,
  MemoryFact,
  MemoryFactPatch,
  MemorySnapshotRecord,
  MemoryToolGateEvent,
  patchMemoryFact,
  renameMemoryPaths,
  rollbackMemorySnapshot,
} from '../../lib/api/cfpmMemory';

const CONTEXT_STRATEGY_CONFIG_KEY = 'AGIME_CONTEXT_COMPACTION_STRATEGY';
const LEGACY_CONTEXT_STRATEGY = 'legacy_segmented';
const CFPM_CONTEXT_STRATEGY = 'cfpm_memory_v1';
const CFPM_RUNTIME_VISIBILITY_CONFIG_KEY = 'AGIME_CFPM_RUNTIME_VISIBILITY';
const CFPM_TOOL_GATE_VISIBILITY_CONFIG_KEY = 'AGIME_CFPM_TOOL_GATE_VISIBILITY';
const MEMORY_AUTO_REFRESH_MS = 5000;
const MEMORY_AUTO_REFRESH_EDIT_SUSPEND_MS = 4000;

type ContextStrategy = typeof LEGACY_CONTEXT_STRATEGY | typeof CFPM_CONTEXT_STRATEGY;
type CfpmRuntimeVisibility = 'off' | 'brief' | 'debug';

const CFPM_RUNTIME_VISIBILITY_ORDER: CfpmRuntimeVisibility[] = ['off', 'brief', 'debug'];

const MEMORY_CATEGORY_OPTIONS = ['goal', 'verified_action', 'artifact', 'invalid_path', 'open_item', 'note'];
const MEMORY_STATUS_OPTIONS: MemoryFact['status'][] = [
  'active',
  'stale',
  'superseded',
  'forgotten',
];

function normalizeContextStrategy(value: unknown): ContextStrategy {
  if (typeof value !== 'string') {
    return LEGACY_CONTEXT_STRATEGY;
  }

  const normalized = value.trim().toLowerCase();
  if (
    normalized === 'cfpm' ||
    normalized === 'cfpm_memory' ||
    normalized === 'cfpm_memory_v1' ||
    normalized === 'progressive' ||
    normalized === 'progressive_memory' ||
    normalized === 'new'
  ) {
    return CFPM_CONTEXT_STRATEGY;
  }

  return LEGACY_CONTEXT_STRATEGY;
}

function normalizeCfpmRuntimeVisibility(value: unknown): CfpmRuntimeVisibility {
  if (typeof value !== 'string') {
    return 'brief';
  }

  const normalized = value.trim().toLowerCase();
  if (normalized === 'off' || normalized === 'none' || normalized === 'disable' || normalized === '0') {
    return 'off';
  }
  if (normalized === 'debug' || normalized === 'verbose' || normalized === 'full' || normalized === '2') {
    return 'debug';
  }
  return 'brief';
}

const alertIcons: Record<AlertType, React.ReactNode> = {
  [AlertType.Error]: <IoIosCloseCircle className="h-5 w-5" />,
  [AlertType.Warning]: <IoIosWarning className="h-5 w-5" />,
  [AlertType.Info]: <IoIosInformationCircle className="h-5 w-5" />,
};

interface AlertBoxProps {
  alert: Alert;
  className?: string;
  compactButtonEnabled?: boolean;
}

const alertStyles: Record<AlertType, string> = {
  [AlertType.Error]: 'bg-[#d7040e] text-white',
  [AlertType.Warning]: 'bg-[#cc4b03] text-white',
  [AlertType.Info]: 'dark:bg-white dark:text-black bg-black text-white',
};

export const AlertBox = ({ alert, className }: AlertBoxProps) => {
  const { t } = useTranslation('chat');
  const { read } = useConfig();

  const [isEditingThreshold, setIsEditingThreshold] = useState(false);
  const [loadedThreshold, setLoadedThreshold] = useState<number>(0.8);
  const [thresholdValue, setThresholdValue] = useState(80);
  const [isSaving, setIsSaving] = useState(false);
  const [contextStrategy, setContextStrategy] =
    useState<ContextStrategy>(LEGACY_CONTEXT_STRATEGY);
  const [isSavingStrategy, setIsSavingStrategy] = useState(false);
  const [cfpmRuntimeVisibility, setCfpmRuntimeVisibility] =
    useState<CfpmRuntimeVisibility>('brief');
  const [isSavingCfpmRuntimeVisibility, setIsSavingCfpmRuntimeVisibility] = useState(false);
  const [cfpmToolGateVisibility, setCfpmToolGateVisibility] =
    useState<CfpmRuntimeVisibility>('brief');
  const [isSavingCfpmToolGateVisibility, setIsSavingCfpmToolGateVisibility] = useState(false);

  const [memoryFacts, setMemoryFacts] = useState<MemoryFact[]>([]);
  const [memoryCandidates, setMemoryCandidates] = useState<MemoryCandidate[]>([]);
  const [memorySnapshots, setMemorySnapshots] = useState<MemorySnapshotRecord[]>([]);
  const [memoryToolGates, setMemoryToolGates] = useState<MemoryToolGateEvent[]>([]);
  const [isLoadingMemory, setIsLoadingMemory] = useState(false);
  const [memoryError, setMemoryError] = useState<string | null>(null);
  const [savingFactIds, setSavingFactIds] = useState<Record<string, boolean>>({});
  const [dirtyFactIds, setDirtyFactIds] = useState<Record<string, true>>({});
  const [newFactCategory, setNewFactCategory] = useState<string>('note');
  const [newFactContent, setNewFactContent] = useState('');
  const [creatingFact, setCreatingFact] = useState(false);
  const [renameFromPath, setRenameFromPath] = useState('');
  const [renameToPath, setRenameToPath] = useState('');
  const [renamingPaths, setRenamingPaths] = useState(false);
  const [selectedSnapshotId, setSelectedSnapshotId] = useState<number | null>(null);
  const [rollingBackSnapshot, setRollingBackSnapshot] = useState(false);
  const [isMemoryDialogOpen, setIsMemoryDialogOpen] = useState(false);
  const memoryLoadingRef = useRef(false);
  const dirtyFactIdsRef = useRef<Record<string, true>>({});
  const lastMemoryEditAtRef = useRef<number>(0);

  const isCfpmMode = contextStrategy === CFPM_CONTEXT_STRATEGY;
  const canUseMemoryPanel = isCfpmMode && Boolean(alert.sessionId);

  const compactButtonLabel = isCfpmMode ? t('alert.compactNowNew') : t('alert.compactNow');

  const progressRatio = useMemo(() => {
    if (!alert.progress) {
      return 0;
    }
    return alert.progress.current / alert.progress.total;
  }, [alert.progress]);

  const loadMemoryData = useCallback(async (options?: { preserveLocalEdits?: boolean; clearDirtyOnApply?: boolean }) => {
    if (!alert.sessionId || memoryLoadingRef.current) {
      return;
    }

    memoryLoadingRef.current = true;
    setIsLoadingMemory(true);
    setMemoryError(null);
    try {
      const [factsResult, candidatesResult, snapshotsResult, toolGatesResult] = await Promise.all([
        listMemoryFacts(alert.sessionId),
        listMemoryCandidates(alert.sessionId, { limit: 200 }),
        listMemorySnapshots(alert.sessionId),
        listMemoryToolGateEvents(alert.sessionId, { limit: 40 }),
      ]);

      if (factsResult.error) {
        throw new Error(String(factsResult.error));
      }
      if (candidatesResult.error) {
        throw new Error(String(candidatesResult.error));
      }
      if (snapshotsResult.error) {
        throw new Error(String(snapshotsResult.error));
      }
      if (toolGatesResult.error) {
        throw new Error(String(toolGatesResult.error));
      }

      const preserveLocalEdits = options?.preserveLocalEdits ?? true;
      const clearDirtyOnApply = options?.clearDirtyOnApply ?? false;
      const incomingFacts = factsResult.data ?? [];
      const dirtyIds = dirtyFactIdsRef.current;
      const hasDirty = Object.keys(dirtyIds).length > 0;

      if (preserveLocalEdits && hasDirty) {
        setMemoryFacts((prev) => {
          const prevById = new Map(prev.map((fact) => [fact.id, fact]));
          const merged = incomingFacts.map((fact) => {
            if (dirtyIds[fact.id]) {
              return prevById.get(fact.id) ?? fact;
            }
            return fact;
          });

          for (const [factId, localFact] of prevById.entries()) {
            if (dirtyIds[factId] && !merged.some((fact) => fact.id === factId)) {
              merged.unshift(localFact);
            }
          }

          return merged;
        });
      } else {
        setMemoryFacts(incomingFacts);
        if (clearDirtyOnApply) {
          setDirtyFactIds({});
        }
      }
      setMemoryCandidates(candidatesResult.data ?? []);
      const snapshots = snapshotsResult.data ?? [];
      setMemorySnapshots(snapshots);
      setMemoryToolGates(toolGatesResult.data ?? []);
      setSelectedSnapshotId((prev) => {
        if (prev && snapshots.some((s) => s.id === prev)) {
          return prev;
        }
        return snapshots.length > 0 ? snapshots[0].id : null;
      });
    } catch (error) {
      console.error('Error loading CFPM memory panel data:', error);
      setMemoryError(
        t('alert.memoryLoadFailed', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      memoryLoadingRef.current = false;
      setIsLoadingMemory(false);
    }
  }, [alert.sessionId, t]);

  useEffect(() => {
    dirtyFactIdsRef.current = dirtyFactIds;
  }, [dirtyFactIds]);

  useEffect(() => {
    const loadSettings = async () => {
      try {
        const [threshold, strategy, runtimeVisibility, toolGateVisibility] = await Promise.all([
          read('AGIME_AUTO_COMPACT_THRESHOLD', false),
          read(CONTEXT_STRATEGY_CONFIG_KEY, false),
          read(CFPM_RUNTIME_VISIBILITY_CONFIG_KEY, false),
          read(CFPM_TOOL_GATE_VISIBILITY_CONFIG_KEY, false),
        ]);

        if (threshold !== undefined && threshold !== null && typeof threshold === 'number') {
          setLoadedThreshold(threshold);
          setThresholdValue(Math.max(1, Math.round(threshold * 100)));
        }

        setContextStrategy(normalizeContextStrategy(strategy));
        setCfpmRuntimeVisibility(normalizeCfpmRuntimeVisibility(runtimeVisibility));
        setCfpmToolGateVisibility(normalizeCfpmRuntimeVisibility(toolGateVisibility));
      } catch (err) {
        console.error('Error fetching context compaction settings:', err);
      }
    };

    loadSettings();
  }, [read]);

  useEffect(() => {
    if (canUseMemoryPanel) {
      void loadMemoryData({ preserveLocalEdits: true });
    } else {
      setMemoryFacts([]);
      setMemoryCandidates([]);
      setMemorySnapshots([]);
      setMemoryToolGates([]);
      setSelectedSnapshotId(null);
      setMemoryError(null);
      setDirtyFactIds({});
      lastMemoryEditAtRef.current = 0;
    }
  }, [canUseMemoryPanel, loadMemoryData]);

  useEffect(() => {
    if (!canUseMemoryPanel) {
      return;
    }

    const intervalId = window.setInterval(() => {
      const hasDirtyEdits = Object.keys(dirtyFactIdsRef.current).length > 0;
      const inEditCooldown =
        Date.now() - lastMemoryEditAtRef.current < MEMORY_AUTO_REFRESH_EDIT_SUSPEND_MS;
      if (hasDirtyEdits || inEditCooldown) {
        return;
      }
      void loadMemoryData({ preserveLocalEdits: true });
    }, MEMORY_AUTO_REFRESH_MS);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [canUseMemoryPanel, loadMemoryData]);

  const currentThreshold = loadedThreshold;

  const handleSaveThreshold = async () => {
    if (isSaving) return;

    let validThreshold = Math.max(1, Math.min(100, thresholdValue));
    if (validThreshold !== thresholdValue) {
      setThresholdValue(validThreshold);
    }

    setIsSaving(true);
    try {
      const newThreshold = validThreshold / 100;

      await upsertConfig({
        body: {
          key: 'AGIME_AUTO_COMPACT_THRESHOLD',
          value: newThreshold,
          is_secret: false,
        },
      });

      setIsEditingThreshold(false);
      setLoadedThreshold(newThreshold);

      if (alert.onThresholdChange) {
        alert.onThresholdChange(newThreshold);
      }
    } catch (error) {
      console.error('Error saving threshold:', error);
      window.alert(
        t('alert.failedToSaveThreshold', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      setIsSaving(false);
    }
  };

  const handleToggleContextStrategy = async () => {
    if (isSavingStrategy) return;

    const nextStrategy =
      contextStrategy === CFPM_CONTEXT_STRATEGY ? LEGACY_CONTEXT_STRATEGY : CFPM_CONTEXT_STRATEGY;

    setIsSavingStrategy(true);
    try {
      await upsertConfig({
        body: {
          key: CONTEXT_STRATEGY_CONFIG_KEY,
          value: nextStrategy,
          is_secret: false,
        },
      });
      setContextStrategy(nextStrategy);
    } catch (error) {
      console.error('Error saving context strategy:', error);
      window.alert(
        t('alert.failedToSaveStrategy', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      setIsSavingStrategy(false);
    }
  };

  const cycleVisibility = async (
    configKey: string,
    current: CfpmRuntimeVisibility,
    setCurrent: React.Dispatch<React.SetStateAction<CfpmRuntimeVisibility>>,
    isSaving: boolean,
    setIsSaving: React.Dispatch<React.SetStateAction<boolean>>,
    errorKey: string,
  ) => {
    if (isSaving) return;

    const nextVisibility =
      CFPM_RUNTIME_VISIBILITY_ORDER[
        (CFPM_RUNTIME_VISIBILITY_ORDER.indexOf(current) + 1) % CFPM_RUNTIME_VISIBILITY_ORDER.length
      ];

    setIsSaving(true);
    try {
      await upsertConfig({ body: { key: configKey, value: nextVisibility, is_secret: false } });
      setCurrent(nextVisibility);
    } catch (error) {
      console.error(`Error saving ${configKey}:`, error);
      window.alert(
        t(errorKey, { error: error instanceof Error ? error.message : t('alert.unknownError') })
      );
    } finally {
      setIsSaving(false);
    }
  };

  const handleCycleCfpmRuntimeVisibility = () =>
    cycleVisibility(
      CFPM_RUNTIME_VISIBILITY_CONFIG_KEY, cfpmRuntimeVisibility, setCfpmRuntimeVisibility,
      isSavingCfpmRuntimeVisibility, setIsSavingCfpmRuntimeVisibility,
      'alert.failedToSaveCfpmRuntimeVisibility',
    );

  const handleCycleCfpmToolGateVisibility = () =>
    cycleVisibility(
      CFPM_TOOL_GATE_VISIBILITY_CONFIG_KEY, cfpmToolGateVisibility, setCfpmToolGateVisibility,
      isSavingCfpmToolGateVisibility, setIsSavingCfpmToolGateVisibility,
      'alert.failedToSaveCfpmToolGateVisibility',
    );

  const handleMemoryFactLocalChange = (
    factId: string,
    key: keyof MemoryFactPatch,
    value: string | boolean
  ) => {
    lastMemoryEditAtRef.current = Date.now();
    setDirtyFactIds((prev) => {
      if (prev[factId]) {
        return prev;
      }
      return { ...prev, [factId]: true };
    });
    setMemoryFacts((prev) =>
      prev.map((fact) => {
        if (fact.id !== factId) {
          return fact;
        }
        return {
          ...fact,
          [key]: value,
        } as MemoryFact;
      })
    );
  };

  const handleSaveMemoryFact = async (fact: MemoryFact) => {
    if (!alert.sessionId) {
      return;
    }
    setSavingFactIds((prev) => ({ ...prev, [fact.id]: true }));
    try {
      const patch: MemoryFactPatch = {
        category: fact.category,
        content: fact.content,
        status: fact.status,
        pinned: fact.pinned,
      };
      const result = await patchMemoryFact(alert.sessionId, fact.id, patch);
      if (result.error) {
        throw new Error(String(result.error));
      }
      if (result.data) {
        setMemoryFacts((prev) => prev.map((item) => (item.id === fact.id ? result.data! : item)));
        setDirtyFactIds((prev) => {
          if (!prev[fact.id]) {
            return prev;
          }
          const next = { ...prev };
          delete next[fact.id];
          return next;
        });
      }
    } catch (error) {
      console.error('Error saving memory fact:', error);
      setMemoryError(
        t('alert.memoryUpdateFailed', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      setSavingFactIds((prev) => ({ ...prev, [fact.id]: false }));
    }
  };

  const handleCreateMemoryFact = async () => {
    if (!alert.sessionId || creatingFact) {
      return;
    }
    const content = newFactContent.trim();
    if (!content) {
      return;
    }

    setCreatingFact(true);
    setMemoryError(null);
    try {
      const result = await createMemoryFact(alert.sessionId, {
        category: newFactCategory,
        content,
        source: 'user',
        pinned: false,
      });
      if (result.error) {
        throw new Error(String(result.error));
      }
      if (result.data) {
        setMemoryFacts((prev) => [result.data!, ...prev]);
      }
      setNewFactContent('');
      setNewFactCategory('note');
    } catch (error) {
      console.error('Error creating memory fact:', error);
      setMemoryError(
        t('alert.memoryCreateFailed', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      setCreatingFact(false);
    }
  };

  const handleRenamePaths = async () => {
    if (!alert.sessionId || renamingPaths) {
      return;
    }

    const fromPath = renameFromPath.trim();
    const toPath = renameToPath.trim();
    if (!fromPath || !toPath || fromPath === toPath) {
      return;
    }

    setRenamingPaths(true);
    setMemoryError(null);
    try {
      const result = await renameMemoryPaths(alert.sessionId, {
        fromPath,
        toPath,
      });
      if (result.error) {
        throw new Error(String(result.error));
      }
      await loadMemoryData({ preserveLocalEdits: false, clearDirtyOnApply: true });
    } catch (error) {
      console.error('Error renaming memory paths:', error);
      setMemoryError(
        t('alert.memoryRenameFailed', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      setRenamingPaths(false);
    }
  };

  const handleRollbackSnapshot = async () => {
    if (!alert.sessionId || rollingBackSnapshot || !selectedSnapshotId) {
      return;
    }

    setRollingBackSnapshot(true);
    setMemoryError(null);
    try {
      const result = await rollbackMemorySnapshot(alert.sessionId, selectedSnapshotId);
      if (result.error) {
        throw new Error(String(result.error));
      }
      await loadMemoryData({ preserveLocalEdits: false, clearDirtyOnApply: true });
    } catch (error) {
      console.error('Error rolling back memory snapshot:', error);
      setMemoryError(
        t('alert.memoryRollbackFailed', {
          error: error instanceof Error ? error.message : t('alert.unknownError'),
        })
      );
    } finally {
      setRollingBackSnapshot(false);
    }
  };

  const closestDotIndex = useMemo(() => {
    if (!isCfpmMode && currentThreshold > 0 && currentThreshold <= 1) {
      let minDistance = Infinity;
      let closest = -1;
      for (let i = 0; i < 30; i++) {
        const dotPos = i / 29;
        const distance = Math.abs(dotPos - currentThreshold);
        if (distance < minDistance) {
          minDistance = distance;
          closest = i;
        }
      }
      return closest;
    }
    return -1;
  }, [currentThreshold, isCfpmMode]);

  const candidateAcceptedCount = useMemo(
    () => memoryCandidates.filter((candidate) => candidate.decision === 'accepted').length,
    [memoryCandidates]
  );
  const candidateRejected = useMemo(
    () => memoryCandidates.filter((candidate) => candidate.decision === 'rejected'),
    [memoryCandidates]
  );

  const formatCandidateReason = useCallback(
    (reason: string) =>
      t(`alert.memoryCandidateReasonCode.${reason}`, {
        defaultValue: reason,
      }),
    [t]
  );

  const formatMemoryConfidence = useCallback(
    (confidence?: number) => {
      if (typeof confidence !== 'number' || !Number.isFinite(confidence)) {
        return t('alert.memoryMetaUnknown');
      }
      const pct = Math.round(Math.max(0, Math.min(1, confidence)) * 100);
      return `${pct}%`;
    },
    [t]
  );

  const formatMemoryDateTime = useCallback(
    (value?: string | null) => {
      if (!value) {
        return t('alert.memoryMetaUnknown');
      }
      const parsed = new Date(value);
      if (Number.isNaN(parsed.getTime())) {
        return value;
      }
      return parsed.toLocaleString();
    },
    [t]
  );

  function visibilityLabel(visibility: CfpmRuntimeVisibility): string {
    if (visibility === 'off') return t('alert.cfpmRuntimeOff');
    if (visibility === 'debug') return t('alert.cfpmRuntimeDebug');
    return t('alert.cfpmRuntimeBrief');
  }

  const renderVisibilityToggle = (
    labelKey: string,
    visibility: CfpmRuntimeVisibility,
    saving: boolean,
    onCycle: () => void,
  ) => (
    <div className="flex items-center justify-center gap-2 min-h-[20px]">
      <span className="text-[10px] opacity-70">{t(labelKey)}</span>
      <button
        type="button"
        onClick={(e) => { e.preventDefault(); e.stopPropagation(); void onCycle(); }}
        disabled={saving}
        className={cn(
          'px-2 py-0.5 rounded text-[10px] border border-current/30 transition-colors',
          saving ? 'opacity-50 cursor-not-allowed' : 'hover:bg-white/10 cursor-pointer'
        )}
      >
        {visibilityLabel(visibility)}
      </button>
    </div>
  );

  const renderMemoryManagementContent = (expanded: boolean) => {
    const sectionBorder = expanded ? 'border-borderSubtle' : 'border-white/15';
    const sectionCls = cn('border-t pt-2 flex flex-col gap-1', sectionBorder);
    const itemCls = cn('rounded border px-1.5 py-1 text-[10px] break-words', expanded ? 'border-borderSubtle bg-background-muted' : 'border-white/15 bg-white/5');
    const inputCls = cn('text-[10px] border rounded px-1 py-0.5', expanded ? 'bg-background-default border-borderSubtle' : 'bg-white/10 border-current/25');
    const actionBtnBase = 'text-[10px] px-2 rounded border hover:bg-white/10 disabled:opacity-50 disabled:cursor-not-allowed';
    const actionBtnBorder = expanded ? 'border-borderSubtle hover:bg-background-muted' : 'border-current/30';
    const actionBtnCls = cn(actionBtnBase, 'py-1', actionBtnBorder);
    const actionBtnSmCls = cn(actionBtnBase, 'py-0.5', actionBtnBorder);
    const listCls = (maxH: string) => cn('overflow-y-auto flex flex-col gap-1 pr-1', maxH);

    return (
    <div
      className={cn(
        'p-2 rounded border border-white/20 bg-white/5 flex flex-col gap-2',
        expanded && 'bg-background-default text-text-default border-borderSubtle'
      )}
    >
      {memoryError && (
        <div className={cn('text-[10px] break-words', expanded ? 'text-red-500' : 'text-red-200')}>
          {memoryError}
        </div>
      )}

      <div className="flex items-center gap-1 text-[10px] opacity-80">
        <span>
          {t('alert.memoryFactsCount', {
            count: memoryFacts.length,
          })}
        </span>
        <span>·</span>
        <span>
          {t('alert.memoryCandidatesCount', {
            count: memoryCandidates.length,
          })}
        </span>
        <span>·</span>
        <span>
          {t('alert.memorySnapshotsCount', {
            count: memorySnapshots.length,
          })}
        </span>
        <span>·</span>
        <span>
          {t('alert.memoryToolGatesCount', {
            count: memoryToolGates.length,
          })}
        </span>
      </div>

      <div className="flex items-center gap-1 text-[10px] opacity-80">
        <span>
          {t('alert.memoryCandidateAcceptedCount', {
            count: candidateAcceptedCount,
          })}
        </span>
        <span>·</span>
        <span>
          {t('alert.memoryCandidateRejectedCount', {
            count: candidateRejected.length,
          })}
        </span>
      </div>

      <div className={sectionCls}>
        <span className="text-[10px] opacity-80">{t('alert.memoryCandidateRejectedTitle')}</span>
        <div className={listCls(expanded ? 'max-h-32' : 'max-h-24')}>
          {candidateRejected.length === 0 ? (
            <div className="text-[10px] opacity-70">{t('alert.memoryCandidateEmpty')}</div>
          ) : (
            candidateRejected.slice(0, 12).map((candidate) => (
              <div
                key={candidate.id}
                className={itemCls}
              >
                <div className="opacity-90">
                  [{candidate.category}] {candidate.content}
                </div>
                <div className="opacity-70">
                  {t('alert.memoryCandidateReason', {
                    reason: formatCandidateReason(candidate.reason),
                  })}
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      <div className={sectionCls}>
        <span className="text-[10px] opacity-80">{t('alert.memoryToolGatesTitle')}</span>
        <div className={listCls(expanded ? 'max-h-32' : 'max-h-24')}>
          {memoryToolGates.length === 0 ? (
            <div className="text-[10px] opacity-70">{t('alert.memoryToolGatesEmpty')}</div>
          ) : (
            memoryToolGates.slice(0, 12).map((event, index) => (
              <div
                key={`${event.createdTimestamp}-${event.tool}-${event.path}-${index}`}
                className={itemCls}
              >
                <div className="opacity-90">
                  {t('alert.memoryToolGateSummary', {
                    tool: event.tool,
                    target: event.target,
                    path: event.path,
                  })}
                </div>
                {event.verbosity === 'debug' && (
                  <div className="opacity-70 mt-0.5">
                    {t('alert.memoryToolGateDebug', {
                      command: event.rewrittenCommand || event.originalCommand,
                    })}
                  </div>
                )}
              </div>
            ))
          )}
        </div>
      </div>

      <div className={listCls(expanded ? 'max-h-[45vh]' : 'max-h-44').replace('gap-1', 'gap-2')}>
        {memoryFacts.length === 0 ? (
          <div className="text-[10px] opacity-70">{t('alert.memoryEmpty')}</div>
        ) : (
          memoryFacts.map((fact) => (
            <div
              key={fact.id}
              className={cn(
                'p-1.5 rounded border',
                expanded ? 'border-borderSubtle bg-background-muted' : 'border-white/15 bg-white/5'
              )}
            >
              <div className="grid grid-cols-2 gap-1 mb-1">
                <select
                  value={fact.category}
                  onChange={(e) => handleMemoryFactLocalChange(fact.id, 'category', e.target.value)}
                  className={inputCls}
                >
                  {MEMORY_CATEGORY_OPTIONS.map((category) => (
                    <option key={category} value={category} className="text-black">
                      {category}
                    </option>
                  ))}
                </select>
                <select
                  value={fact.status}
                  onChange={(e) =>
                    handleMemoryFactLocalChange(fact.id, 'status', e.target.value as MemoryFact['status'])
                  }
                  className={inputCls}
                >
                  {MEMORY_STATUS_OPTIONS.map((status) => (
                    <option key={status} value={status} className="text-black">
                      {status}
                    </option>
                  ))}
                </select>
              </div>

              <input
                type="text"
                value={fact.content}
                onChange={(e) => handleMemoryFactLocalChange(fact.id, 'content', e.target.value)}
                className={cn('w-full py-1 mb-1', inputCls)}
              />

              <div className="mb-1 text-[10px] opacity-70 break-words">
                <div>
                  {t('alert.memoryMetaConfidence', {
                    value: formatMemoryConfidence(fact.confidence),
                  })}{' '}
                  ·{' '}
                  {t('alert.memoryMetaEvidence', {
                    count:
                      typeof fact.evidenceCount === 'number' && Number.isFinite(fact.evidenceCount)
                        ? fact.evidenceCount
                        : 1,
                  })}
                </div>
                <div>
                  {t('alert.memoryMetaValidatedAt', {
                    value: formatMemoryDateTime(fact.lastValidatedAt),
                  })}
                </div>
                {fact.validationCommand && (
                  <div className="truncate" title={fact.validationCommand}>
                    {t('alert.memoryMetaCommand', {
                      command: fact.validationCommand,
                    })}
                  </div>
                )}
              </div>

              <div className="flex items-center justify-between">
                <label className="text-[10px] flex items-center gap-1 opacity-80">
                  <input
                    type="checkbox"
                    checked={fact.pinned}
                    onChange={(e) => handleMemoryFactLocalChange(fact.id, 'pinned', e.target.checked)}
                  />
                  {t('alert.memoryPinned')}
                </label>
                <button
                  type="button"
                  className={actionBtnSmCls}
                  onClick={() => {
                    void handleSaveMemoryFact(fact);
                  }}
                  disabled={Boolean(savingFactIds[fact.id])}
                >
                  {savingFactIds[fact.id] ? t('alert.savingMemory') : t('alert.saveMemory')}
                </button>
              </div>
            </div>
          ))
        )}
      </div>

      <div className={sectionCls}>
        <span className="text-[10px] opacity-80">{t('alert.createMemoryFact')}</span>
        <div className="flex gap-1">
          <select
            value={newFactCategory}
            onChange={(e) => setNewFactCategory(e.target.value)}
            className={cn('w-28 py-1', inputCls)}
          >
            {MEMORY_CATEGORY_OPTIONS.map((category) => (
              <option key={category} value={category} className="text-black">
                {category}
              </option>
            ))}
          </select>
          <input
            type="text"
            value={newFactContent}
            onChange={(e) => setNewFactContent(e.target.value)}
            placeholder={t('alert.createMemoryPlaceholder')}
            className={cn('flex-1 py-1', inputCls)}
          />
          <button
            type="button"
            className={actionBtnCls}
            onClick={() => {
              void handleCreateMemoryFact();
            }}
            disabled={creatingFact || !newFactContent.trim()}
          >
            {creatingFact ? t('alert.creatingMemory') : t('alert.createMemory')}
          </button>
        </div>
      </div>

      <div className={sectionCls}>
        <span className="text-[10px] opacity-80">{t('alert.pathRenameTitle')}</span>
        <div className="flex gap-1">
          <input
            type="text"
            value={renameFromPath}
            onChange={(e) => setRenameFromPath(e.target.value)}
            placeholder={t('alert.pathRenameFrom')}
            className={cn('flex-1 py-1', inputCls)}
          />
          <input
            type="text"
            value={renameToPath}
            onChange={(e) => setRenameToPath(e.target.value)}
            placeholder={t('alert.pathRenameTo')}
            className={cn('flex-1 py-1', inputCls)}
          />
          <button
            type="button"
            className={actionBtnCls}
            onClick={() => {
              void handleRenamePaths();
            }}
            disabled={
              renamingPaths ||
              !renameFromPath.trim() ||
              !renameToPath.trim() ||
              renameFromPath.trim() === renameToPath.trim()
            }
          >
            {renamingPaths ? t('alert.pathRenaming') : t('alert.pathRenameRun')}
          </button>
        </div>
      </div>

      <div className={sectionCls}>
        <span className="text-[10px] opacity-80">{t('alert.memoryRollbackTitle')}</span>
        <div className="flex gap-1">
          <select
            value={selectedSnapshotId ?? ''}
            onChange={(e) => setSelectedSnapshotId(Number(e.target.value) || null)}
            className={cn('flex-1 py-1', inputCls)}
            disabled={memorySnapshots.length === 0}
          >
            {memorySnapshots.length === 0 ? (
              <option value="" className="text-black">
                {t('alert.memoryNoSnapshots')}
              </option>
            ) : (
              memorySnapshots.map((snapshot) => (
                <option key={snapshot.id} value={snapshot.id} className="text-black">
                  #{snapshot.id} · {snapshot.factCount} · {snapshot.reason}
                </option>
              ))
            )}
          </select>
          <button
            type="button"
            className={actionBtnCls}
            onClick={() => {
              void handleRollbackSnapshot();
            }}
            disabled={!selectedSnapshotId || rollingBackSnapshot || memorySnapshots.length === 0}
          >
            {rollingBackSnapshot ? t('alert.memoryRollingBack') : t('alert.memoryRollbackRun')}
          </button>
        </div>
      </div>
    </div>
  );
  };

  return (
    <>
      <div
      className={cn('flex flex-col gap-2 px-3 py-3', alertStyles[alert.type], className)}
      onMouseDown={(e) => {
        e.stopPropagation();
      }}
      >
      {alert.progress ? (
        <div className="flex flex-col gap-2">
          <span className="text-[11px]">{alert.message}</span>

          {!isCfpmMode ? (
            <div className="flex items-center justify-center gap-1 min-h-[20px]">
              {isEditingThreshold ? (
                <>
                  <span className="text-[10px] opacity-70">{t('alert.autoCompactAt')}</span>
                  <input
                    type="number"
                    min="1"
                    max="100"
                    step="1"
                    value={thresholdValue}
                    onChange={(e) => {
                      const val = parseInt(e.target.value, 10);
                      if (e.target.value === '') {
                        setThresholdValue(1);
                      } else if (!isNaN(val)) {
                        setThresholdValue(Math.max(1, Math.min(100, val)));
                      }
                    }}
                    onBlur={(e) => {
                      const val = parseInt(e.target.value, 10);
                      if (isNaN(val) || val < 1) {
                        setThresholdValue(1);
                      } else if (val > 100) {
                        setThresholdValue(100);
                      }
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        void handleSaveThreshold();
                      } else if (e.key === 'Escape') {
                        setIsEditingThreshold(false);
                        const resetValue = Math.round(currentThreshold * 100);
                        setThresholdValue(Math.max(1, resetValue));
                      }
                    }}
                    onFocus={(e) => {
                      e.target.select();
                    }}
                    className="w-12 px-1 text-[10px] bg-white/10 border border-current/30 rounded outline-none text-center focus:bg-white/20 focus:border-current/50 transition-colors"
                    disabled={isSaving}
                    autoFocus
                  />
                  <span className="text-[10px] opacity-70">%</span>
                  <button
                    type="button"
                    onMouseDown={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      void handleSaveThreshold();
                    }}
                    disabled={isSaving}
                    className="p-1 hover:opacity-60 transition-opacity cursor-pointer relative z-50"
                    style={{ minWidth: '20px', minHeight: '20px', pointerEvents: 'auto' }}
                  >
                    <FaSave className="w-3 h-3" />
                  </button>
                </>
              ) : (
                <>
                  <span className="text-[10px] opacity-70">
                    {t('alert.autoCompactAt')} {Math.round(currentThreshold * 100)}%
                  </span>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      setIsEditingThreshold(true);
                    }}
                    className="p-1 hover:opacity-60 transition-opacity cursor-pointer relative z-10"
                    style={{ minWidth: '20px', minHeight: '20px' }}
                  >
                    <FaPencilAlt className="w-3 h-3 opacity-70" />
                  </button>
                </>
              )}
            </div>
          ) : (
            <div className="flex items-center justify-center min-h-[20px]">
              <span className="text-[10px] opacity-70">{t('alert.memoryModeHint')}</span>
            </div>
          )}

          <div className="flex items-center justify-center gap-2 min-h-[20px]">
            <span className="text-[10px] opacity-70">{t('alert.contextStrategy')}</span>
            <button
              type="button"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                void handleToggleContextStrategy();
              }}
              disabled={isSavingStrategy}
              className={cn(
                'px-2 py-0.5 rounded text-[10px] border border-current/30 transition-colors',
                isSavingStrategy ? 'opacity-50 cursor-not-allowed' : 'hover:bg-white/10 cursor-pointer'
              )}
            >
              {isCfpmMode ? t('alert.strategyNew') : t('alert.strategyLegacy')}
            </button>
          </div>

          {isCfpmMode && (
            <>
              {renderVisibilityToggle(
                'alert.cfpmRuntimeVisibility', cfpmRuntimeVisibility,
                isSavingCfpmRuntimeVisibility, handleCycleCfpmRuntimeVisibility,
              )}
              {renderVisibilityToggle(
                'alert.cfpmToolGateVisibility', cfpmToolGateVisibility,
                isSavingCfpmToolGateVisibility, handleCycleCfpmToolGateVisibility,
              )}
            </>
          )}

          <div className="flex justify-between w-full relative">
            {[...Array(30)].map((_, i) => {
              const dotPosition = i / 29;
              const isActive = dotPosition <= progressRatio;
              const isThresholdDot = i === closestDotIndex;
              const progressPercentage = Math.round(progressRatio * 100);

              let progressColor: string;
              if (progressPercentage <= 50) progressColor = 'bg-green-500';
              else if (progressPercentage <= 75) progressColor = 'bg-yellow-500';
              else if (progressPercentage <= 90) progressColor = 'bg-orange-500';
              else progressColor = 'bg-red-500';

              return (
                <div
                  key={i}
                  className={cn(
                    'rounded-full transition-all relative',
                    isThresholdDot ? 'h-[6px] w-[6px] -mt-[2px]' : 'h-[2px] w-[2px]',
                    isActive ? progressColor : 'bg-gray-300 dark:bg-gray-600'
                  )}
                />
              );
            })}
          </div>

          <div className="flex justify-between items-baseline text-[11px]">
            <div className="flex gap-1 items-baseline">
              <span className="dark:text-black/60 text-white/60">
                {alert.progress.current >= 1000
                  ? `${(alert.progress.current / 1000).toFixed(1)}k`
                  : alert.progress.current}
              </span>
              <span className="dark:text-black/40 text-white/40">
                {Math.round(progressRatio * 100)}%
              </span>
            </div>
            <span className="dark:text-black/60 text-white/60">
              {alert.progress.total >= 1000
                ? `${(alert.progress.total / 1000).toFixed(0)}k`
                : alert.progress.total}
            </span>
          </div>

          {canUseMemoryPanel && (
            <div className="mt-1 flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <span className="text-[10px] font-medium">{t('alert.memoryPanelTitle')}</span>
                <div className="flex items-center gap-1">
                  <button
                    type="button"
                    className="text-[10px] px-2 py-0.5 rounded border border-current/30 hover:bg-white/10"
                    onClick={() => {
                      void loadMemoryData();
                    }}
                    disabled={isLoadingMemory}
                  >
                    {isLoadingMemory ? t('alert.loadingMemory') : t('alert.refreshMemory')}
                  </button>
                  <button
                    type="button"
                    className="text-[10px] px-2 py-0.5 rounded border border-current/30 hover:bg-white/10"
                    onClick={() => setIsMemoryDialogOpen(true)}
                  >
                    {t('alert.openMemoryPanel')}
                  </button>
                </div>
              </div>
              {renderMemoryManagementContent(false)}
            </div>
          )}

          {alert.showCompactButton && alert.onCompact && (
            <button
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                alert.onCompact!();
              }}
              disabled={alert.compactButtonDisabled}
              className={cn(
                'flex items-center gap-1.5 text-[11px] outline-none mt-1',
                alert.compactButtonDisabled ? 'opacity-50 cursor-not-allowed' : 'hover:opacity-80 cursor-pointer'
              )}
            >
              {alert.compactIcon}
              <span>{compactButtonLabel}</span>
            </button>
          )}
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <div className="flex-shrink-0">{alertIcons[alert.type]}</div>
          <div className="flex flex-col gap-2 flex-1">
            <span className="text-[11px] break-words whitespace-pre-line">{alert.message}</span>
            {alert.action && (
              <a
                role="button"
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  alert.action?.onClick();
                }}
                className="text-[11px] text-left underline hover:opacity-80 cursor-pointer outline-none"
              >
                {alert.action.text}
              </a>
            )}
          </div>
        </div>
      )}
      </div>
      {canUseMemoryPanel && (
        <Dialog open={isMemoryDialogOpen} onOpenChange={setIsMemoryDialogOpen}>
          <DialogContent className="sm:max-w-5xl max-h-[86vh] overflow-hidden flex flex-col">
            <DialogHeader>
              <DialogTitle>{t('alert.memoryPanelTitle')}</DialogTitle>
              <DialogDescription>{t('alert.memoryPanelDescription')}</DialogDescription>
            </DialogHeader>
            <div className="overflow-y-auto pr-1">{renderMemoryManagementContent(true)}</div>
          </DialogContent>
        </Dialog>
      )}
    </>
  );
};
