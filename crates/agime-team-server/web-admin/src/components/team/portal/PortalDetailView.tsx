import { useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, Globe, Copy, Check, Eye, Settings, BarChart3, Trash2, RefreshCw, FolderTree, Folder, FileText, ChevronUp, Activity, Loader2 } from 'lucide-react';
import { Button } from '../../ui/button';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import {
  portalApi,
  type PortalDetail,
  type PortalStats,
  type UpdatePortalRequest,
  type PortalFileEntry,
  type PortalFileContentResponse,
} from '../../../api/portal';
import { chatApi } from '../../../api/chat';
import { agentApi, BUILTIN_EXTENSIONS, type TeamAgent } from '../../../api/agent';
import { documentApi, type DocumentSummary } from '../../../api/documents';
import { ChatConversation, type ChatRuntimeEvent } from '../../chat/ChatConversation';
import { useToast } from '../../../contexts/ToastContext';

interface PortalDetailViewProps {
  teamId: string;
  portalId: string;
  canManage: boolean;
  onBack: () => void;
}

type RightTab = 'preview' | 'files' | 'activity' | 'analytics' | 'settings';

type RuntimeTimelineItem = ChatRuntimeEvent & {
  id: string;
};

type RuntimeExtensionOption = {
  id: string;
  label: string;
  description?: string;
  source: 'builtin' | 'custom';
};

const BUILTIN_RUNTIME_NAME: Record<string, string> = {
  computer_controller: 'computercontroller',
  auto_visualiser: 'autovisualiser',
};

function toRuntimeExtensionName(id: string): string {
  return BUILTIN_RUNTIME_NAME[id] || id;
}

function getRuntimeExtensionOptions(agent: TeamAgent): RuntimeExtensionOption[] {
  const options: RuntimeExtensionOption[] = [];
  const seen = new Set<string>();

  for (const ext of agent.enabled_extensions || []) {
    if (!ext.enabled) continue;
    const runtimeName = toRuntimeExtensionName(ext.extension);
    if (seen.has(runtimeName)) continue;
    seen.add(runtimeName);
    const meta = BUILTIN_EXTENSIONS.find(x => x.id === ext.extension);
    options.push({
      id: runtimeName,
      label: meta?.name || ext.extension,
      description: meta?.description,
      source: 'builtin',
    });
  }

  for (const ext of agent.custom_extensions || []) {
    if (!ext.enabled || !ext.name) continue;
    const runtimeName = ext.name.trim();
    if (!runtimeName || seen.has(runtimeName)) continue;
    seen.add(runtimeName);
    options.push({
      id: runtimeName,
      label: ext.name,
      description: ext.type === 'stdio' ? 'Custom stdio MCP' : 'Custom remote MCP',
      source: 'custom',
    });
  }

  return options;
}

function formatBytes(size?: number | null): string {
  if (typeof size !== 'number' || Number.isNaN(size) || size < 0) return '';
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function normalizeWorkspacePath(input?: string | null): string {
  if (!input) return '';
  return input
    .replace(/\\/g, '/')
    .replace(/\/+/g, '/')
    .replace(/\/+$/, '')
    .toLowerCase();
}

function resolveCodingAgentId(portal?: PortalDetail | null): string | null {
  if (!portal) return null;
  return portal.codingAgentId || portal.agentId || portal.serviceAgentId || null;
}

function resolveServiceAgentId(portal?: PortalDetail | null): string | null {
  if (!portal) return null;
  return portal.serviceAgentId || portal.agentId || portal.codingAgentId || null;
}

export function PortalDetailView({ teamId, portalId, canManage, onBack }: PortalDetailViewProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [portal, setPortal] = useState<PortalDetail | null>(null);
  const [stats, setStats] = useState<PortalStats | null>(null);
  const [rightTab, setRightTab] = useState<RightTab>('preview');
  const [loading, setLoading] = useState(true);
  const [copied, setCopied] = useState(false);
  const [copiedTest, setCopiedTest] = useState(false);
  const [codingAgent, setCodingAgent] = useState<TeamAgent | null>(null);
  const [policyAgent, setPolicyAgent] = useState<TeamAgent | null>(null);
  const [chatSessionId, setChatSessionId] = useState<string | null>(null);
  const [chatProcessing, setChatProcessing] = useState(false);
  const [runtimeEvents, setRuntimeEvents] = useState<RuntimeTimelineItem[]>([]);
  const [previewKey, setPreviewKey] = useState(0);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [publishLoading, setPublishLoading] = useState(false);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  // --- File tree state ---
  const [filePath, setFilePath] = useState('');
  const [fileParentPath, setFileParentPath] = useState<string | null>(null);
  const [fileEntries, setFileEntries] = useState<PortalFileEntry[]>([]);
  const [loadingFiles, setLoadingFiles] = useState(false);
  const [fileError, setFileError] = useState('');
  const [selectedFilePath, setSelectedFilePath] = useState('');
  const [selectedFile, setSelectedFile] = useState<PortalFileContentResponse | null>(null);
  const [loadingFileContent, setLoadingFileContent] = useState(false);
  const [fileContentError, setFileContentError] = useState('');

  // --- Settings edit state ---
  const [editCodingAgentId, setEditCodingAgentId] = useState<string | null>(null);
  const [editServiceAgentId, setEditServiceAgentId] = useState<string | null>(null);
  const [editAgentPrompt, setEditAgentPrompt] = useState('');
  const [editWelcomeMsg, setEditWelcomeMsg] = useState('');
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [savingSettings, setSavingSettings] = useState(false);

  // --- Document selector state ---
  const [allDocuments, setAllDocuments] = useState<DocumentSummary[]>([]);
  const [selectedDocIds, setSelectedDocIds] = useState<string[]>([]);
  const [selectedExtensions, setSelectedExtensions] = useState<string[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);
  const portalSessionStorageKey = `portal_chat_session:v2:${teamId}:${portalId}`;
  const runtimeEventsStoragePrefix = `portal_runtime_events:v1:${teamId}:${portalId}:`;
  const clearPersistedSession = useCallback(() => {
    setChatSessionId(prev => {
      if (prev) {
        try {
          window.localStorage.removeItem(`${runtimeEventsStoragePrefix}${prev}`);
        } catch {}
      }
      return null;
    });
    setRuntimeEvents([]);
    try {
      window.localStorage.removeItem(portalSessionStorageKey);
    } catch {}
  }, [portalSessionStorageKey, runtimeEventsStoragePrefix]);

  const load = async () => {
    try {
      setLoading(true);
      const p = await portalApi.get(teamId, portalId);
      setPortal(p);
      setSelectedDocIds(p.boundDocumentIds || []);
      const codingAgentId = resolveCodingAgentId(p);
      const serviceAgentId = resolveServiceAgentId(p);
      let loadedCodingAgent: TeamAgent | null = null;
      let loadedServiceAgent: TeamAgent | null = null;

      if (codingAgentId) {
        try {
          loadedCodingAgent = await agentApi.getAgent(codingAgentId);
          setCodingAgent(loadedCodingAgent);
        } catch {
          setCodingAgent(null);
        }
      } else {
        setCodingAgent(null);
      }

      if (serviceAgentId) {
        if (loadedCodingAgent && codingAgentId === serviceAgentId) {
          loadedServiceAgent = loadedCodingAgent;
        } else {
          try {
            loadedServiceAgent = await agentApi.getAgent(serviceAgentId);
          } catch {
            loadedServiceAgent = null;
          }
        }
      }
      setPolicyAgent(loadedServiceAgent);

      if (loadedServiceAgent) {
        const extDefaults = getRuntimeExtensionOptions(loadedServiceAgent).map(o => o.id);
        const skillDefaults = (loadedServiceAgent.assigned_skills || [])
          .filter(s => s.enabled)
          .map(s => s.skill_id);
        setSelectedExtensions(p.allowedExtensions ?? extDefaults);
        setSelectedSkillIds(p.allowedSkillIds ?? skillDefaults);
      } else {
        setSelectedExtensions(p.allowedExtensions ?? []);
        setSelectedSkillIds(p.allowedSkillIds ?? []);
      }
      // Init settings edit state
      setEditCodingAgentId(codingAgentId);
      setEditServiceAgentId(serviceAgentId);
      setEditAgentPrompt(p.agentSystemPrompt || '');
      setEditWelcomeMsg(p.agentWelcomeMessage || '');
    } catch {
      addToast('error', t('laboratory.loadError'));
    } finally {
      setLoading(false);
    }
  };

  const loadStats = async () => {
    try {
      const s = await portalApi.getStats(teamId, portalId);
      setStats(s);
    } catch {
      addToast('error', t('laboratory.loadError'));
    }
  };

  const loadFiles = useCallback(async (path = '', withLoading = true) => {
    if (!portal?.projectPath) {
      setFileEntries([]);
      setFileParentPath(null);
      setFilePath('');
      return;
    }
    if (withLoading) setLoadingFiles(true);
    setFileError('');
    try {
      const res = await portalApi.listFiles(teamId, portalId, path);
      setFileEntries(res.entries || []);
      setFilePath(res.path || '');
      setFileParentPath(res.parentPath ?? null);
    } catch (e: any) {
      const msg = e?.message || t('laboratory.operationError');
      setFileError(msg);
    } finally {
      if (withLoading) setLoadingFiles(false);
    }
  }, [portal?.projectPath, teamId, portalId, t]);

  const loadFileContent = useCallback(async (path: string, withLoading = true) => {
    if (!portal?.projectPath || !path) {
      setSelectedFilePath('');
      setSelectedFile(null);
      setFileContentError('');
      return;
    }
    setSelectedFilePath(path);
    if (withLoading) setLoadingFileContent(true);
    setFileContentError('');
    try {
      const res = await portalApi.getFile(teamId, portalId, path);
      setSelectedFile(res);
    } catch (e: any) {
      const msg = e?.message || t('laboratory.operationError');
      setFileContentError(msg);
      setSelectedFile(null);
    } finally {
      if (withLoading) setLoadingFileContent(false);
    }
  }, [portal?.projectPath, teamId, portalId, t]);

  // Load agents list for settings selector
  useEffect(() => {
    agentApi.listAgents(teamId).then(res => setAgents(res.items || [])).catch(() => {});
  }, [teamId]);

  // Load documents list for settings selector
  useEffect(() => {
    documentApi.listDocuments(teamId, 1, 200).then(res => setAllDocuments(res.items || [])).catch(() => {});
  }, [teamId]);

  // Keep policy agent in sync with selected agent in Settings.
  useEffect(() => {
    const effectivePolicyAgentId = editServiceAgentId || editCodingAgentId;
    if (!effectivePolicyAgentId) {
      setPolicyAgent(null);
      setSelectedExtensions([]);
      setSelectedSkillIds([]);
      return;
    }
    agentApi.getAgent(effectivePolicyAgentId).then(a => {
      setPolicyAgent(a);
      // When user switches to a different agent, reset policy defaults to all available.
      if (!portal || effectivePolicyAgentId !== resolveServiceAgentId(portal)) {
        setSelectedExtensions(getRuntimeExtensionOptions(a).map(o => o.id));
        setSelectedSkillIds((a.assigned_skills || []).filter(s => s.enabled).map(s => s.skill_id));
      }
    }).catch(() => setPolicyAgent(null));
  }, [editServiceAgentId, editCodingAgentId, portal]);

  useEffect(() => { load(); }, [teamId, portalId]);
  useEffect(() => { if (rightTab === 'analytics') loadStats(); }, [rightTab]);
  // Restore the last session for this portal so chat history survives navigation.
  useEffect(() => {
    try {
      const saved = window.localStorage.getItem(portalSessionStorageKey);
      setChatSessionId(saved || null);
    } catch {
      setChatSessionId(null);
    }
  }, [portalSessionStorageKey]);
  useEffect(() => {
    if (!chatSessionId || loading || !portal) return;
    const codingAgentId = resolveCodingAgentId(portal);
    if (!codingAgentId || !portal.projectPath) {
      clearPersistedSession();
      return;
    }
    let cancelled = false;
    chatApi.getSession(chatSessionId).then(detail => {
      if (cancelled) return;
      const allowlist = (detail.allowed_extensions || [])
        .map(item => item.trim().toLowerCase())
        .filter(Boolean);
      const allowed = new Set(
        allowlist
      );
      const samePortal =
        (detail.portal_id != null && detail.portal_id === portal.id) ||
        (detail.portal_slug != null && detail.portal_slug === portal.slug);
      const hasRequiredExtensions = allowlist.length === 0 || allowed.has('developer');
      const valid =
        detail.agent_id === codingAgentId &&
        detail.portal_restricted === true &&
        samePortal &&
        normalizeWorkspacePath(detail.workspace_path) === normalizeWorkspacePath(portal.projectPath) &&
        hasRequiredExtensions;
      if (!valid) clearPersistedSession();
    }).catch((err) => {
      if (cancelled) return;
      // Keep local persisted session on transient fetch errors so users do not
      // lose chat history just because a single validation request failed.
      console.warn('Portal session validation skipped due to transient error:', err);
    });
    return () => { cancelled = true; };
  }, [chatSessionId, clearPersistedSession, loading, portal]);
  useEffect(() => {
    if (loading || !portal) return;
    if (resolveCodingAgentId(portal) && portal.projectPath) return;
    clearPersistedSession();
  }, [clearPersistedSession, loading, portal]);
  useEffect(() => {
    if (!chatSessionId) {
      setRuntimeEvents([]);
      return;
    }
    try {
      const raw = window.localStorage.getItem(`${runtimeEventsStoragePrefix}${chatSessionId}`);
      if (!raw) {
        setRuntimeEvents([]);
        return;
      }
      const parsed = JSON.parse(raw);
      if (!Array.isArray(parsed)) {
        setRuntimeEvents([]);
        return;
      }
      const hydrated = parsed
        .filter((item: any) =>
          item &&
          typeof item.id === 'string' &&
          typeof item.kind === 'string' &&
          typeof item.text === 'string' &&
          typeof item.ts === 'number'
        )
        .slice(-300) as RuntimeTimelineItem[];
      setRuntimeEvents(hydrated);
    } catch {
      setRuntimeEvents([]);
    }
  }, [chatSessionId, runtimeEventsStoragePrefix]);
  useEffect(() => {
    if (rightTab !== 'files') return;
    loadFiles(filePath || '');
  }, [rightTab, loadFiles]);
  useEffect(() => {
    if (portal?.projectPath) return;
    setSelectedFilePath('');
    setSelectedFile(null);
    setFileContentError('');
  }, [portal?.projectPath]);

  // Auto-refresh file tree while agent is running (vibe coding visibility)
  useEffect(() => {
    if (rightTab !== 'files' || !chatProcessing) return;
    const timer = window.setInterval(() => {
      loadFiles(filePath || '', false);
      if (selectedFilePath) {
        loadFileContent(selectedFilePath, false);
      }
    }, 2000);
    return () => window.clearInterval(timer);
  }, [rightTab, chatProcessing, filePath, loadFiles, selectedFilePath, loadFileContent]);

  const handlePublish = async () => {
    if (!portal) return;
    setPublishLoading(true);
    try {
      if (portal.status === 'published') {
        const updated = await portalApi.unpublish(teamId, portalId);
        setPortal(updated);
        addToast('success', t('laboratory.unpublishSuccess'));
      } else {
        const updated = await portalApi.publish(teamId, portalId);
        setPortal(updated);
        addToast('success', t('laboratory.publishSuccess'));
      }
    } catch {
      addToast('error', t('laboratory.operationError'));
    } finally {
      setPublishLoading(false);
    }
  };

  const handleDelete = () => {
    setShowDeleteConfirm(true);
  };

  const confirmDelete = async () => {
    try {
      await portalApi.delete(teamId, portalId);
      clearPersistedSession();
      addToast('success', t('laboratory.deleteSuccess'));
      onBack();
    } catch (e: any) {
      addToast('error', e?.message || t('laboratory.operationError'));
    } finally {
      setShowDeleteConfirm(false);
    }
  };

  const handleSessionCreated = useCallback((sessionId: string) => {
    setChatSessionId(prev => {
      if (prev && prev !== sessionId) {
        try {
          window.localStorage.removeItem(`${runtimeEventsStoragePrefix}${prev}`);
        } catch {}
      }
      return sessionId;
    });
    setRuntimeEvents([]);
    try {
      window.localStorage.setItem(portalSessionStorageKey, sessionId);
      window.localStorage.removeItem(`${runtimeEventsStoragePrefix}${sessionId}`);
    } catch {}
  }, [portalSessionStorageKey, runtimeEventsStoragePrefix]);

  const createPortalCodingSession = useCallback(async () => {
    const codingAgentId = resolveCodingAgentId(portal);
    if (!codingAgentId) {
      throw new Error(t('laboratory.noAgentSelected'));
    }
    if (!portal?.projectPath) {
      throw new Error(t('laboratory.noProjectPath'));
    }
    // Pre-check: agent must have developer extension
    if (codingAgent) {
      const hasDev = codingAgent.enabled_extensions?.some(
        (e) => e.enabled && e.extension === 'developer',
      );
      if (!hasDev) {
        throw new Error(t('laboratory.agentMissingDeveloper', 'Agent does not have Developer extension enabled'));
      }
    }
    const res = await chatApi.createPortalCodingSession(teamId, portalId);
    return res.session_id;
  }, [portal, portalId, teamId, codingAgent, t]);

  const copyUrl = () => {
    if (!portal) return;
    const targetUrl =
      portal.status === 'published'
        ? (portal.publicUrl || portal.testPublicUrl || `${window.location.origin}/p/${portal.slug}`)
        : portal.previewUrl;
    navigator.clipboard.writeText(targetUrl);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const copyTestUrl = () => {
    if (!portal || portal.status !== 'published') return;
    const targetUrl = portal.testPublicUrl || `${window.location.origin}/p/${portal.slug}`;
    navigator.clipboard.writeText(targetUrl);
    setCopiedTest(true);
    setTimeout(() => setCopiedTest(false), 2000);
  };

  // Refresh preview when Agent updates portal via tools
  const handleToolResult = useCallback((_toolName: string, _result: string, success: boolean) => {
    if (!success) return;
    // Refresh portal data and preview on any tool success
    portalApi.get(teamId, portalId).then(setPortal).catch(() => {});
    setPreviewKey(k => k + 1);
    if (rightTab === 'files') {
      loadFiles(filePath || '', false);
      if (selectedFilePath) {
        loadFileContent(selectedFilePath, false);
      }
    }
  }, [teamId, portalId, rightTab, loadFiles, filePath, selectedFilePath, loadFileContent]);

  const handleRuntimeEvent = useCallback((event: ChatRuntimeEvent) => {
    setRuntimeEvents(prev => {
      const next: RuntimeTimelineItem = {
        ...event,
        id: `${event.ts}-${Math.random().toString(36).slice(2, 8)}`,
      };
      const merged = [...prev, next];
      const trimmed = merged.length <= 300 ? merged : merged.slice(merged.length - 300);
      if (chatSessionId) {
        try {
          window.localStorage.setItem(
            `${runtimeEventsStoragePrefix}${chatSessionId}`,
            JSON.stringify(trimmed)
          );
        } catch {}
      }
      return trimmed;
    });

    if (event.kind === 'workspace_changed') {
      setPreviewKey(k => k + 1);
      if (rightTab === 'files') {
        loadFiles(filePath || '', false);
        if (selectedFilePath) {
          loadFileContent(selectedFilePath, false);
        }
      }
    }
  }, [chatSessionId, runtimeEventsStoragePrefix, rightTab, loadFiles, filePath, selectedFilePath, loadFileContent]);

  const handleSaveSettings = async () => {
    if (!portal) return;
    setSavingSettings(true);
    try {
      const prompt = editAgentPrompt.trim();
      const welcome = editWelcomeMsg.trim();
      const effectiveCodingAgentId = editCodingAgentId || null;
      const effectiveServiceAgentId = editServiceAgentId || editCodingAgentId || null;
      const req: UpdatePortalRequest = {
        codingAgentId: effectiveCodingAgentId,
        serviceAgentId: effectiveServiceAgentId,
        agentEnabled: !!effectiveServiceAgentId,
        agentSystemPrompt: prompt ? editAgentPrompt : null,
        agentWelcomeMessage: welcome ? editWelcomeMsg : null,
        boundDocumentIds: selectedDocIds,
        allowedExtensions: selectedExtensions,
        allowedSkillIds: selectedSkillIds,
      };
      const updated = await portalApi.update(teamId, portalId, req);
      setPortal(updated);
      setEditCodingAgentId(resolveCodingAgentId(updated));
      setEditServiceAgentId(resolveServiceAgentId(updated));
      // Reload coding agent if changed
      const currentCodingAgentId = resolveCodingAgentId(portal);
      if (effectiveCodingAgentId && effectiveCodingAgentId !== currentCodingAgentId) {
        try {
          const a = await agentApi.getAgent(effectiveCodingAgentId);
          setCodingAgent(a);
        } catch { setCodingAgent(null); }
      } else if (!effectiveCodingAgentId) {
        setCodingAgent(null);
      }

      // Reload policy agent if changed
      const currentServiceAgentId = resolveServiceAgentId(portal);
      if (effectiveServiceAgentId && effectiveServiceAgentId !== currentServiceAgentId) {
        try {
          const a = await agentApi.getAgent(effectiveServiceAgentId);
          setPolicyAgent(a);
        } catch { setPolicyAgent(null); }
      } else if (!effectiveServiceAgentId) {
        setPolicyAgent(null);
      }
      // Only force new session when the coding agent actually changed.
      // Other settings (prompt, docs, extensions) are synced by the backend.
      const prevCodingAgentId = resolveCodingAgentId(portal);
      if (effectiveCodingAgentId !== prevCodingAgentId) {
        clearPersistedSession();
      }
      addToast('success', t('laboratory.saveSuccess'));
    } catch {
      addToast('error', t('laboratory.operationError'));
    } finally {
      setSavingSettings(false);
    }
  };

  const toggleDocId = (docId: string) => {
    setSelectedDocIds(prev =>
      prev.includes(docId) ? prev.filter(id => id !== docId) : [...prev, docId]
    );
  };

  const toggleExtension = (ext: string) => {
    setSelectedExtensions(prev =>
      prev.includes(ext) ? prev.filter(id => id !== ext) : [...prev, ext]
    );
  };

  const toggleSkillId = (skillId: string) => {
    setSelectedSkillIds(prev =>
      prev.includes(skillId) ? prev.filter(id => id !== skillId) : [...prev, skillId]
    );
  };

  if (loading || !portal) {
    return (
      <div className="flex items-center justify-center py-24">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary" />
      </div>
    );
  }

  const statusColor = portal.status === 'published'
    ? 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400'
    : 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400';

  const rightTabs: { key: RightTab; icon: React.ReactNode; label: string }[] = [
    { key: 'preview', icon: <Eye className="w-3.5 h-3.5" />, label: t('laboratory.preview') },
    { key: 'files', icon: <FolderTree className="w-3.5 h-3.5" />, label: t('laboratory.files', 'Files') },
    { key: 'activity', icon: <Activity className="w-3.5 h-3.5" />, label: t('laboratory.activity', 'Activity') },
    { key: 'analytics', icon: <BarChart3 className="w-3.5 h-3.5" />, label: t('laboratory.analytics') },
    { key: 'settings', icon: <Settings className="w-3.5 h-3.5" />, label: t('laboratory.settings') },
  ];

  const codingAgentId = resolveCodingAgentId(portal);
  // Always use public route so the chat widget gets injected into HTML
  const previewBaseUrl = `/p/${portal.slug}`;
  const canPreviewViaIframe = !!portal.projectPath;
  const extensionOptions = policyAgent ? getRuntimeExtensionOptions(policyAgent) : [];
  const skillOptions = policyAgent
    ? (policyAgent.assigned_skills || []).filter(s => s.enabled)
    : [];
  const timelineEvents = [...runtimeEvents].reverse();
  const selectedFileUrl = selectedFilePath
    ? `${previewBaseUrl}/${selectedFilePath.split('/').map(s => encodeURIComponent(s)).join('/')}`
    : '';

  const runtimeBadgeClass = (kind: ChatRuntimeEvent['kind']) => {
    if (kind === 'toolcall' || kind === 'toolresult') return 'bg-blue-500';
    if (kind === 'workspace_changed') return 'bg-emerald-500';
    if (kind === 'compaction') return 'bg-amber-500';
    if (kind === 'goal') return 'bg-rose-500';
    if (kind === 'done') return 'bg-slate-500';
    if (kind === 'connection') return 'bg-violet-500';
    return 'bg-primary';
  };

  return (
    <div className="flex flex-col h-[calc(100vh-40px)]">
      {/* Header bar */}
      <div className="flex items-center gap-3 px-4 py-2 border-b shrink-0">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="w-4 h-4" />
        </Button>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <h2 className="text-base font-semibold truncate">{portal.name}</h2>
            <span className={`text-xs px-2 py-0.5 rounded-full ${statusColor}`}>
              {t(`laboratory.status.${portal.status}`)}
            </span>
          </div>
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Globe className="w-3 h-3" />
            <span>/p/{portal.slug}</span>
            <button
              onClick={copyUrl}
              className="hover:text-foreground"
              title={t('laboratory.copyUrl')}
            >
              {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
            </button>
            {portal.status === 'published' &&
              portal.publicUrl &&
              portal.testPublicUrl &&
              portal.publicUrl !== portal.testPublicUrl && (
                <button
                  onClick={copyTestUrl}
                  className="px-1 py-0.5 rounded border border-border hover:text-foreground"
                  title={t('laboratory.copyTestUrl', 'Copy test URL (IP:port)')}
                >
                  {copiedTest ? <Check className="w-3 h-3" /> : 'IP'}
                </button>
              )}
          </div>
        </div>
        {canManage && (
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant={portal.status === 'published' ? 'outline' : 'default'}
              onClick={handlePublish}
              disabled={publishLoading}
            >
              {publishLoading && <Loader2 className="w-4 h-4 animate-spin mr-1.5" />}
              {portal.status === 'published' ? t('laboratory.unpublish') : t('laboratory.publish')}
            </Button>
            <Button size="sm" variant="ghost" onClick={handleDelete}>
              <Trash2 className="w-4 h-4 text-[hsl(var(--destructive))]" />
            </Button>
          </div>
        )}
      </div>

      {/* Split pane: left = chat, right = preview/settings */}
      <div className="flex flex-1 min-h-0">
        {/* Left panel: Agent chat */}
        <div className="w-[420px] border-r flex flex-col shrink-0">
          {codingAgent && codingAgentId ? (
            <ChatConversation
              sessionId={chatSessionId}
              agentId={codingAgentId}
              agentName={codingAgent.name}
              agent={codingAgent}
              teamId={teamId}
              initialAttachedDocIds={portal.boundDocumentIds}
              createSession={createPortalCodingSession}
              onSessionCreated={handleSessionCreated}
              onToolResult={handleToolResult}
              onProcessingChange={setChatProcessing}
              onRuntimeEvent={handleRuntimeEvent}
              onError={(message) => addToast('error', message)}
            />
          ) : (
            <div className="flex-1 flex flex-col items-center justify-center text-muted-foreground gap-3 p-6">
              <p className="text-sm text-center">{t('laboratory.noCodingAgentSelected', 'Please select a coding agent first')}</p>
              <Button size="sm" variant="outline" onClick={() => setRightTab('settings')}>
                {t('laboratory.codingAgentSelect', 'Coding Agent')}
              </Button>
            </div>
          )}
        </div>

        {/* Right panel: tabs */}
        <div className="flex-1 flex flex-col min-w-0">
          {/* Right tab bar */}
          <div className="flex items-center gap-1 px-3 border-b shrink-0">
            {rightTabs.map((tab) => (
              <button
                key={tab.key}
                className={`flex items-center gap-1 px-2.5 py-2 text-xs border-b-2 transition-colors ${
                  rightTab === tab.key
                    ? 'border-primary text-primary'
                    : 'border-transparent text-muted-foreground hover:text-foreground'
                }`}
                onClick={() => setRightTab(tab.key)}
              >
                {tab.icon}
                {tab.label}
              </button>
            ))}
            {rightTab === 'preview' && (
              <button
                className="ml-auto text-muted-foreground hover:text-foreground p-1"
                onClick={() => setPreviewKey(k => k + 1)}
                title={t('laboratory.refreshPreview')}
              >
                <RefreshCw className="w-3.5 h-3.5" />
              </button>
            )}
          </div>

          {/* Right tab content */}
          <div className="flex-1 overflow-auto">
            {rightTab === 'preview' && (
              <div className="h-full">
                {canPreviewViaIframe ? (
                  <iframe
                    ref={iframeRef}
                    key={previewKey}
                    src={previewBaseUrl}
                    className="w-full h-full border-0"
                    title="Portal Preview"
                  />
                ) : (
                  <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
                    {t('laboratory.agentHint')}
                  </div>
                )}
              </div>
            )}

            {rightTab === 'files' && (
              <div className="h-full flex flex-col">
                <div className="px-3 py-2 border-b flex items-center gap-2 text-xs">
                  <button
                    className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-50"
                    onClick={() => fileParentPath != null && loadFiles(fileParentPath)}
                    disabled={loadingFiles || fileParentPath == null}
                    title={t('laboratory.parentDir', 'Parent Directory')}
                  >
                    <ChevronUp className="w-3.5 h-3.5" />
                  </button>
                  <button
                    className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-50"
                    onClick={() => loadFiles(filePath || '')}
                    disabled={loadingFiles}
                    title={t('laboratory.refreshFiles', 'Refresh files')}
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${loadingFiles ? 'animate-spin' : ''}`} />
                  </button>
                  <span className="font-mono text-muted-foreground truncate">
                    /{filePath || ''}
                  </span>
                  {chatProcessing && (
                    <span className="ml-auto text-[11px] text-muted-foreground">
                      {t('laboratory.autoRefreshing', 'Auto refreshing while Agent is running')}
                    </span>
                  )}
                </div>
                <div className="flex-1 min-h-0">
                  {!portal.projectPath ? (
                    <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                      {t('laboratory.noProjectPath', 'Project path not initialized')}
                    </div>
                  ) : (
                    <div className="h-full grid grid-cols-1 md:grid-cols-[340px_1fr]">
                      <div className="border-b md:border-b-0 md:border-r overflow-auto">
                        {fileError ? (
                          <div className="p-4 text-sm text-[hsl(var(--destructive))]">{fileError}</div>
                        ) : loadingFiles && fileEntries.length === 0 ? (
                          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                            {t('common.loading')}
                          </div>
                        ) : fileEntries.length === 0 ? (
                          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                            {t('laboratory.emptyFolder', 'Folder is empty')}
                          </div>
                        ) : (
                          <div className="divide-y">
                            {fileEntries.map(entry => (
                              <button
                                key={entry.path}
                                className={`w-full px-3 py-2 text-left hover:bg-muted/50 flex items-center gap-2 ${
                                  !entry.isDir && selectedFilePath === entry.path ? 'bg-muted/60' : ''
                                }`}
                                onClick={() => {
                                  if (entry.isDir) {
                                    setSelectedFilePath('');
                                    setSelectedFile(null);
                                    setFileContentError('');
                                    loadFiles(entry.path);
                                  } else {
                                    loadFileContent(entry.path);
                                  }
                                }}
                              >
                                {entry.isDir ? (
                                  <Folder className="w-4 h-4 text-blue-500 shrink-0" />
                                ) : (
                                  <FileText className="w-4 h-4 text-muted-foreground shrink-0" />
                                )}
                                <div className="min-w-0 flex-1">
                                  <div className="text-sm truncate">{entry.name}</div>
                                  <div className="text-[11px] text-muted-foreground truncate">
                                    {entry.path}
                                  </div>
                                </div>
                                {!entry.isDir && (
                                  <div className="text-[11px] text-muted-foreground shrink-0 text-right">
                                    <div>{formatBytes(entry.size)}</div>
                                    {entry.modifiedAt && (
                                      <div>{new Date(entry.modifiedAt).toLocaleTimeString()}</div>
                                    )}
                                  </div>
                                )}
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                      <div className="overflow-auto min-h-0">
                        {!selectedFilePath ? (
                          <div className="h-full flex items-center justify-center text-sm text-muted-foreground px-4 text-center">
                            {t('laboratory.selectFileToPreview', 'Select a file to preview its content')}
                          </div>
                        ) : fileContentError ? (
                          <div className="p-4 space-y-3">
                            <p className="text-sm text-[hsl(var(--destructive))]">{fileContentError}</p>
                            <button
                              className="px-3 py-1.5 text-xs border rounded hover:bg-muted"
                              onClick={() => loadFileContent(selectedFilePath)}
                            >
                              {t('common.retry')}
                            </button>
                          </div>
                        ) : loadingFileContent && !selectedFile ? (
                          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                            {t('common.loading')}
                          </div>
                        ) : selectedFile ? (
                          <div className="h-full flex flex-col">
                            <div className="px-3 py-2 border-b flex items-start gap-2">
                              <div className="min-w-0 flex-1">
                                <div className="text-sm font-medium truncate">{selectedFile.name}</div>
                                <div className="text-[11px] text-muted-foreground font-mono truncate">
                                  {selectedFile.path}
                                </div>
                              </div>
                              {loadingFileContent && (
                                <RefreshCw className="w-3.5 h-3.5 animate-spin text-muted-foreground mt-0.5" />
                              )}
                              {selectedFileUrl && (
                                <a
                                  href={selectedFileUrl}
                                  target="_blank"
                                  rel="noreferrer"
                                  className="text-[11px] px-2 py-1 border rounded hover:bg-muted whitespace-nowrap"
                                >
                                  {t('documents.openInNewTab', 'Open in new tab')}
                                </a>
                              )}
                            </div>
                            <div className="px-3 py-2 border-b text-[11px] text-muted-foreground flex flex-wrap gap-x-3 gap-y-1">
                              <span>{selectedFile.contentType}</span>
                              <span>{formatBytes(selectedFile.size)}</span>
                              {selectedFile.modifiedAt && (
                                <span>{new Date(selectedFile.modifiedAt).toLocaleString()}</span>
                              )}
                              {selectedFile.truncated && (
                                <span className="text-amber-600">
                                  {t('laboratory.filePreviewTruncated', 'Preview truncated to first 512 KB')}
                                </span>
                              )}
                            </div>
                            <div className="flex-1 overflow-auto">
                              {selectedFile.isText ? (
                                <pre className="text-xs leading-5 p-3 whitespace-pre-wrap break-words font-mono">
                                  {selectedFile.content || ''}
                                </pre>
                              ) : (
                                <div className="h-full flex items-center justify-center text-sm text-muted-foreground px-4 text-center">
                                  {t('laboratory.binaryPreviewUnavailable', 'Binary file preview is not available')}
                                </div>
                              )}
                            </div>
                          </div>
                        ) : null}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            )}

            {rightTab === 'activity' && (
              <div className="h-full flex flex-col">
                <div className="px-3 py-2 border-b flex items-center gap-2 text-xs">
                  <span className="font-medium">{t('laboratory.activity', 'Activity')}</span>
                  <span className="text-muted-foreground">({runtimeEvents.length})</span>
                  {chatProcessing && (
                    <span className="ml-2 text-muted-foreground">
                      {t('laboratory.activityLive', 'Live updates')}
                    </span>
                  )}
                  <button
                    className="ml-auto px-2 py-1 border rounded hover:bg-muted"
                    onClick={() => {
                      setRuntimeEvents([]);
                      if (chatSessionId) {
                        try {
                          window.localStorage.removeItem(`${runtimeEventsStoragePrefix}${chatSessionId}`);
                        } catch {}
                      }
                    }}
                  >
                    {t('common.reset', 'Reset')}
                  </button>
                </div>
                <div className="flex-1 overflow-auto">
                  {timelineEvents.length === 0 ? (
                    <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                      {t('laboratory.activityEmpty', 'No runtime events yet')}
                    </div>
                  ) : (
                    <div className="divide-y">
                      {timelineEvents.map(item => (
                        <div key={item.id} className="px-3 py-2 flex items-start gap-2">
                          <span className={`mt-1 h-2.5 w-2.5 rounded-full ${runtimeBadgeClass(item.kind)}`} />
                          <div className="min-w-0 flex-1">
                            <div className="text-sm break-words">{item.text}</div>
                            <div className="text-[11px] text-muted-foreground">
                              {new Date(item.ts).toLocaleTimeString()}
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            )}

            {rightTab === 'analytics' && (
              <div className="p-4 grid gap-4 sm:grid-cols-2">
                {[
                  { label: t('laboratory.visitors'), value: stats?.uniqueVisitors ?? '—' },
                  { label: t('laboratory.pageViews'), value: stats?.pageViews ?? '—' },
                  { label: t('laboratory.chatMessages'), value: stats?.chatMessages ?? '—' },
                  { label: t('laboratory.interactions'), value: stats?.totalInteractions ?? '—' },
                ].map((item) => (
                  <div key={item.label} className="border rounded-lg p-4">
                    <p className="text-sm text-muted-foreground">{item.label}</p>
                    <p className="text-2xl font-semibold mt-1">{item.value}</p>
                  </div>
                ))}
              </div>
            )}

            {rightTab === 'settings' && (
              <div className="p-4 space-y-4 max-w-lg">
                {/* Coding agent selector */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.codingAgentSelect', 'Coding Agent')}</label>
                  <select
                    className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm"
                    value={editCodingAgentId || ''}
                    onChange={(e) => setEditCodingAgentId(e.target.value || null)}
                  >
                    <option value="">{t('laboratory.noAgentSelected')}</option>
                    {agents.map(a => (
                      <option key={a.id} value={a.id}>{a.name}{a.model ? ` (${a.model})` : ''}</option>
                    ))}
                  </select>
                  <p className="text-xs text-muted-foreground mt-1">
                    {t('laboratory.codingAgentHint', 'This agent handles portal laboratory coding sessions')}
                  </p>
                </div>

                {/* Service agent selector */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.serviceAgentSelect', 'Service Agent')}</label>
                  <select
                    className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm"
                    value={editServiceAgentId || ''}
                    onChange={(e) => setEditServiceAgentId(e.target.value || null)}
                  >
                    <option value="">{t('laboratory.followCodingAgent', 'Follow coding agent')}</option>
                    {agents.map(a => (
                      <option key={a.id} value={a.id}>{a.name}{a.model ? ` (${a.model})` : ''}</option>
                    ))}
                  </select>
                  <p className="text-xs text-muted-foreground mt-1">
                    {t('laboratory.serviceAgentHint', 'This agent serves external visitors on published portal pages')}
                  </p>
                </div>

                {/* Agent system prompt */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.agentSystemPrompt')}</label>
                  <textarea
                    className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm min-h-[80px] resize-y"
                    value={editAgentPrompt}
                    onChange={(e) => setEditAgentPrompt(e.target.value)}
                    placeholder="Optional system prompt override for this portal..."
                  />
                </div>

                {/* Welcome message */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.agentWelcomeMessage')}</label>
                  <textarea
                    className="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm min-h-[60px] resize-y"
                    value={editWelcomeMsg}
                    onChange={(e) => setEditWelcomeMsg(e.target.value)}
                    placeholder="Welcome message for chat widget..."
                  />
                </div>

                {/* Bound documents selector */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.boundDocs')}</label>
                  <p className="text-xs text-muted-foreground mb-2">
                    {t('laboratory.docsSelected', '{{count}} documents selected', { count: selectedDocIds.length })}
                  </p>
                  <div className="border rounded-md max-h-[200px] overflow-y-auto">
                    {allDocuments.length === 0 ? (
                      <p className="text-sm text-muted-foreground p-3 text-center">
                        {t('laboratory.noDocumentsAvailable', 'No documents available')}
                      </p>
                    ) : (
                      allDocuments.map(doc => (
                        <label
                          key={doc.id}
                          className="flex items-center gap-2 px-3 py-2 hover:bg-muted/50 cursor-pointer border-b last:border-b-0"
                        >
                          <input
                            type="checkbox"
                            checked={selectedDocIds.includes(doc.id)}
                            onChange={() => toggleDocId(doc.id)}
                            className="rounded border-gray-300"
                          />
                          <span className="text-sm truncate flex-1">{doc.display_name || doc.name}</span>
                          <span className="text-xs text-muted-foreground">{doc.mime_type}</span>
                        </label>
                      ))
                    )}
                  </div>
                </div>

                {/* Visitor extension allowlist */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.allowedExtensionsVisitor', 'Allowed Extensions (Visitor)')}</label>
                  <p className="text-xs text-muted-foreground mb-2">
                    {t('laboratory.enabledForExternalUsers', '{{count}} enabled for external users', { count: selectedExtensions.length })}
                  </p>
                  <div className="border rounded-md max-h-[200px] overflow-y-auto">
                    {!(editServiceAgentId || editCodingAgentId) ? (
                      <p className="text-sm text-muted-foreground p-3 text-center">
                        {t('laboratory.selectServiceAgentFirst', 'Select a service agent first')}
                      </p>
                    ) : extensionOptions.length === 0 ? (
                      <p className="text-sm text-muted-foreground p-3 text-center">
                        {t('laboratory.noEnabledExtensionsOnAgent', 'No enabled extensions on this agent')}
                      </p>
                    ) : (
                      extensionOptions.map(ext => (
                        <label
                          key={ext.id}
                          className="flex items-start gap-2 px-3 py-2 hover:bg-muted/50 cursor-pointer border-b last:border-b-0"
                        >
                          <input
                            type="checkbox"
                            checked={selectedExtensions.includes(ext.id)}
                            onChange={() => toggleExtension(ext.id)}
                            className="rounded border-gray-300 mt-0.5"
                          />
                          <span className="flex-1 min-w-0">
                            <span className="text-sm block">{ext.label}</span>
                            <span className="text-xs text-muted-foreground block truncate">
                              {ext.id}{ext.description ? ` - ${ext.description}` : ''}
                            </span>
                          </span>
                          <span className="text-[10px] uppercase text-muted-foreground">{ext.source}</span>
                        </label>
                      ))
                    )}
                  </div>
                </div>

                {/* Visitor skills allowlist */}
                <div>
                  <label className="text-sm font-medium">{t('laboratory.allowedSkillsVisitor', 'Allowed Skills (Visitor)')}</label>
                  <p className="text-xs text-muted-foreground mb-2">
                    {t('laboratory.skillsEnabledForExternalUsers', '{{count}} skills enabled for external users', { count: selectedSkillIds.length })}
                  </p>
                  <div className="border rounded-md max-h-[180px] overflow-y-auto">
                    {!(editServiceAgentId || editCodingAgentId) ? (
                      <p className="text-sm text-muted-foreground p-3 text-center">
                        {t('laboratory.selectServiceAgentFirst', 'Select a service agent first')}
                      </p>
                    ) : skillOptions.length === 0 ? (
                      <p className="text-sm text-muted-foreground p-3 text-center">
                        {t('laboratory.noAssignedSkillsOnAgent', 'No assigned skills on this agent')}
                      </p>
                    ) : (
                      skillOptions.map(skill => (
                        <label
                          key={skill.skill_id}
                          className="flex items-start gap-2 px-3 py-2 hover:bg-muted/50 cursor-pointer border-b last:border-b-0"
                        >
                          <input
                            type="checkbox"
                            checked={selectedSkillIds.includes(skill.skill_id)}
                            onChange={() => toggleSkillId(skill.skill_id)}
                            className="rounded border-gray-300 mt-0.5"
                          />
                          <span className="flex-1 min-w-0">
                            <span className="text-sm block">{skill.name}</span>
                            <span className="text-xs text-muted-foreground block truncate">
                              {skill.skill_id}
                            </span>
                          </span>
                        </label>
                      ))
                    )}
                  </div>
                </div>

                {/* Project path (read-only info) */}
                {portal.projectPath && (
                  <div>
                    <label className="text-sm font-medium">{t('laboratory.projectPath', 'Project Path')}</label>
                    <p className="text-sm text-muted-foreground font-mono bg-muted/50 rounded px-2 py-1 mt-1">
                      {portal.projectPath}
                    </p>
                  </div>
                )}

                {canManage && (
                  <Button onClick={handleSaveSettings} disabled={savingSettings}>
                    {savingSettings && <Loader2 className="w-4 h-4 animate-spin mr-1.5" />}
                    {savingSettings ? t('common.saving') : t('common.save')}
                  </Button>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
      <ConfirmDialog
        open={showDeleteConfirm}
        onOpenChange={setShowDeleteConfirm}
        title={t('laboratory.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDelete}
      />
    </div>
  );
}
