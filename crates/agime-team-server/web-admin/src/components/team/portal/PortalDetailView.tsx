import { useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowLeft, Globe, Copy, Check, Settings, BarChart3, Trash2, RefreshCw, FolderTree, Folder, FileText, ChevronUp, Activity, Loader2, X, Monitor, Tablet, Smartphone, Bot, MessageSquare, Shield, Plus, MessageCircle, Eye } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { Button } from '../../ui/button';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { StatusBadge } from '../../ui/status-badge';
import { LoadingState } from '../../ui/loading-state';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '../../ui/dialog';
import {
  portalApi,
  type AvatarInstanceProjection,
  type PortalDetail,
  type PortalDocumentAccessMode,
  type PortalStats,
  type UpdatePortalRequest,
  type PortalFileEntry,
  type PortalFileContentResponse,
} from '../../../api/portal';
import { chatApi } from '../../../api/chat';
import { agentApi, BUILTIN_EXTENSIONS, type TeamAgent } from '../../../api/agent';
import { documentApi, type DocumentSummary } from '../../../api/documents';
import { DocumentPicker } from '../../documents/DocumentPicker';
import { ChatConversation, type ChatRuntimeEvent } from '../../chat/ChatConversation';
import { useToast } from '../../../contexts/ToastContext';
import { useIsMobile } from '../../../hooks/useMediaQuery';
import { copyText } from '../../../utils/clipboard';
import { formatTime, formatDateTime } from '../../../utils/format';
import {
  classifyPortalServiceAgent,
  describePortalServiceBindingMode,
  formatPortalServiceAgentOptionLabel,
  groupPortalServiceAgents,
} from './serviceAgentBinding';

interface PortalDetailViewProps {
  teamId: string;
  portalId: string;
  canManage: boolean;
  onBack: () => void;
}

type FloatingPanel = 'files' | 'activity' | 'analytics' | null;
type PreviewDevice = 'desktop' | 'tablet' | 'mobile';

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

function toggleItem<T>(prev: T[], item: T): T[] {
  return prev.includes(item) ? prev.filter(x => x !== item) : [...prev, item];
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

function resolveShowChatWidget(portal?: PortalDetail | null): boolean {
  if (!portal) return true;
  const raw = (portal.settings as Record<string, unknown> | undefined)?.showChatWidget;
  return typeof raw === 'boolean' ? raw : true;
}

export function PortalDetailView({ teamId, portalId, canManage, onBack }: PortalDetailViewProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const navigate = useNavigate();
  const isMobile = useIsMobile();
  const [mobileTab, setMobileTab] = useState<'chat' | 'preview' | 'settings'>('chat');
  const [portal, setPortal] = useState<PortalDetail | null>(null);
  const [stats, setStats] = useState<PortalStats | null>(null);
  const [activePanel, setActivePanel] = useState<FloatingPanel>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [previewDevice, setPreviewDevice] = useState<PreviewDevice>('desktop');
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
  const [selectorDialog, setSelectorDialog] = useState<'extensions' | 'skills' | null>(null);
  const [showDocPickerSettings, setShowDocPickerSettings] = useState(false);
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
  const [avatarInstances, setAvatarInstances] = useState<AvatarInstanceProjection[]>([]);
  const [savingSettings, setSavingSettings] = useState(false);

  // --- Document selector state ---
  const [allDocuments, setAllDocuments] = useState<DocumentSummary[]>([]);
  const [selectedDocIds, setSelectedDocIds] = useState<string[]>([]);
  const [selectedExtensions, setSelectedExtensions] = useState<string[]>([]);
  const [selectedSkillIds, setSelectedSkillIds] = useState<string[]>([]);
  const [editDocumentAccessMode, setEditDocumentAccessMode] =
    useState<PortalDocumentAccessMode>('read_only');
  const [editShowChatWidget, setEditShowChatWidget] = useState(true);
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

  const findAgentById = useCallback(
    (id?: string | null) => agents.find(agent => agent.id === id) || null,
    [agents]
  );

  const syncPortalStateFromServer = useCallback(async (withLoading = false) => {
    try {
      if (withLoading) setLoading(true);
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

      // Keep settings editor fully aligned with backend state.
      setEditCodingAgentId(codingAgentId);
      setEditServiceAgentId(serviceAgentId);
      setEditAgentPrompt(p.agentSystemPrompt || '');
      setEditWelcomeMsg(p.agentWelcomeMessage || '');
      setEditDocumentAccessMode(p.documentAccessMode || 'read_only');
      setEditShowChatWidget(resolveShowChatWidget(p));
    } catch {
      if (withLoading) {
        addToast('error', t('ecosystem.loadError'));
      }
    } finally {
      if (withLoading) setLoading(false);
    }
  }, [teamId, portalId, addToast, t]);

  const load = useCallback(async () => {
    await syncPortalStateFromServer(true);
  }, [syncPortalStateFromServer]);

  const loadStats = useCallback(async () => {
    try {
      const s = await portalApi.getStats(teamId, portalId);
      setStats(s);
    } catch {
      addToast('error', t('ecosystem.loadError'));
    }
  }, [teamId, portalId, addToast, t]);

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
      const msg = e?.message || t('ecosystem.operationError');
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
      const msg = e?.message || t('ecosystem.operationError');
      setFileContentError(msg);
      setSelectedFile(null);
    } finally {
      if (withLoading) setLoadingFileContent(false);
    }
  }, [portal?.projectPath, teamId, portalId, t]);

  const refreshOpenFiles = useCallback(() => {
    loadFiles(filePath || '', false);
    if (selectedFilePath) {
      loadFileContent(selectedFilePath, false);
    }
  }, [loadFiles, filePath, selectedFilePath, loadFileContent]);

  // Load agents list for settings selector
  useEffect(() => {
    agentApi.listAgents(teamId).then(res => setAgents(res.items || [])).catch(() => {});
  }, [teamId]);

  useEffect(() => {
    portalApi.listAvatarInstances(teamId).then(setAvatarInstances).catch(() => {});
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

  useEffect(() => { load(); }, [load]);
  useEffect(() => { if (activePanel === 'analytics') loadStats(); }, [activePanel]);
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
    if (activePanel !== 'files') return;
    loadFiles(filePath || '');
  }, [activePanel, loadFiles]);
  useEffect(() => {
    if (portal?.projectPath) return;
    setSelectedFilePath('');
    setSelectedFile(null);
    setFileContentError('');
  }, [portal?.projectPath]);

  // Auto-refresh file tree while agent is running (vibe coding visibility)
  useEffect(() => {
    if (activePanel !== 'files' || !chatProcessing) return;
    const timer = window.setInterval(refreshOpenFiles, 2000);
    return () => window.clearInterval(timer);
  }, [activePanel, chatProcessing, refreshOpenFiles]);

  const handlePublish = async () => {
    if (!portal) return;
    setPublishLoading(true);
    try {
      if (portal.status === 'published') {
        const updated = await portalApi.unpublish(teamId, portalId);
        setPortal(updated);
        addToast('success', t('ecosystem.unpublishSuccess'));
      } else {
        const updated = await portalApi.publish(teamId, portalId);
        setPortal(updated);
        addToast('success', t('ecosystem.publishSuccess'));
      }
    } catch {
      addToast('error', t('ecosystem.operationError'));
    } finally {
      setPublishLoading(false);
    }
  };

  const confirmDelete = async () => {
    try {
      await portalApi.delete(teamId, portalId);
      clearPersistedSession();
      addToast('success', t('ecosystem.deleteSuccess'));
      onBack();
    } catch (e: any) {
      addToast('error', e?.message || t('ecosystem.operationError'));
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
      throw new Error(t('ecosystem.noAgentSelected'));
    }
    if (!portal?.projectPath) {
      throw new Error(t('ecosystem.noProjectPath'));
    }
    // Pre-check: agent must have developer extension
    if (codingAgent) {
      const hasDev = codingAgent.enabled_extensions?.some(
        (e) => e.enabled && e.extension === 'developer',
      );
      if (!hasDev) {
        throw new Error(t('ecosystem.agentMissingDeveloper', 'Agent does not have Developer extension enabled'));
      }
    }
    const res = await chatApi.createPortalCodingSession(teamId, portalId);
    return res.session_id;
  }, [portal, portalId, teamId, codingAgent, t]);

  const copyUrl = async () => {
    if (!portal) return;
    const targetUrl = portal.publicUrl || portal.testPublicUrl || portal.previewUrl;
    if (!targetUrl) return;
    if (await copyText(targetUrl)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const copyTestUrl = async () => {
    if (!portal) return;
    const targetUrl = portal.testPublicUrl;
    if (!targetUrl) return;
    if (await copyText(targetUrl)) {
      setCopiedTest(true);
      setTimeout(() => setCopiedTest(false), 2000);
    }
  };

  // Refresh preview when Agent updates portal via tools
  const handleToolResult = useCallback((toolName: string, _result: string, success: boolean) => {
    if (!success) return;

    const normalizedToolName = (toolName || '').toLowerCase();
    const isPortalConfigMutation =
      normalizedToolName.includes('configure_portal_service_agent') ||
      normalizedToolName.includes('publish_portal');

    // Config mutations must fully resync settings state, not only preview/meta.
    if (isPortalConfigMutation) {
      syncPortalStateFromServer(false).catch(() => {});
    } else {
      // Keep lightweight refresh for normal coding/file tools.
      portalApi.get(teamId, portalId).then(setPortal).catch(() => {});
    }

    setPreviewKey(k => k + 1);
    if (activePanel === 'files') {
      refreshOpenFiles();
    }
  }, [teamId, portalId, activePanel, refreshOpenFiles, syncPortalStateFromServer]);

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
      if (activePanel === 'files') {
        refreshOpenFiles();
      }
    }
  }, [chatSessionId, runtimeEventsStoragePrefix, activePanel, refreshOpenFiles]);

  const handleSaveSettings = async () => {
    if (!portal) return;
    setSavingSettings(true);
    try {
      const prompt = editAgentPrompt.trim();
      const welcome = editWelcomeMsg.trim();
      const effectiveCodingAgentId = editCodingAgentId || null;
      const serviceSourceAgentId = editServiceAgentId || editCodingAgentId || null;
      const serviceSourceAgent = findAgentById(serviceSourceAgentId);
      const serviceBindingMode = classifyPortalServiceAgent(serviceSourceAgent);
      let effectiveServiceAgentId = serviceSourceAgentId;

      if (serviceSourceAgentId) {
        if (!serviceSourceAgent) {
          addToast('error', t('ecosystem.serviceAgentSourceMissing', '未找到选中的服务 Agent。'));
          return;
        }
        switch (serviceBindingMode) {
          case 'clone_general': {
            const dedicated = await agentApi.provisionFromTemplate(serviceSourceAgent.id, {
              name: `${portal.name} ${t('ecosystem.serviceAgentNameSuffix', '服务Agent')}`.trim(),
              agent_domain: 'ecosystem_portal',
              agent_role: 'service',
              template_source_agent_id: serviceSourceAgent.id,
            });
            effectiveServiceAgentId = dedicated.id;
            break;
          }
          case 'direct_ecosystem':
          case 'shared_avatar':
            effectiveServiceAgentId = serviceSourceAgent.id;
            break;
          case 'invalid_avatar_manager':
          case 'invalid_other':
            addToast('error', describePortalServiceBindingMode(t, serviceBindingMode));
            return;
          case 'none':
            break;
        }
      }

      const currentSettings =
        portal.settings &&
        typeof portal.settings === 'object' &&
        !Array.isArray(portal.settings)
          ? (portal.settings as Record<string, unknown>)
          : {};
      const req: UpdatePortalRequest = {
        codingAgentId: effectiveCodingAgentId,
        serviceAgentId: effectiveServiceAgentId,
        agentEnabled: !!effectiveServiceAgentId,
        agentSystemPrompt: prompt ? editAgentPrompt : null,
        agentWelcomeMessage: welcome ? editWelcomeMsg : null,
        boundDocumentIds: selectedDocIds,
        allowedExtensions: selectedExtensions,
        allowedSkillIds: selectedSkillIds,
        documentAccessMode: editDocumentAccessMode,
        settings: {
          ...currentSettings,
          showChatWidget: editShowChatWidget,
        },
      };
      const updated = await portalApi.update(teamId, portalId, req);
      setPortal(updated);
      setEditCodingAgentId(resolveCodingAgentId(updated));
      setEditServiceAgentId(resolveServiceAgentId(updated));
      setEditDocumentAccessMode(updated.documentAccessMode || 'read_only');
      setEditShowChatWidget(resolveShowChatWidget(updated));
      // Reload coding agent if changed
      const prevCodingAgentId = resolveCodingAgentId(portal);
      if (effectiveCodingAgentId && effectiveCodingAgentId !== prevCodingAgentId) {
        try {
          const a = await agentApi.getAgent(effectiveCodingAgentId);
          setCodingAgent(a);
        } catch { setCodingAgent(null); }
      } else if (!effectiveCodingAgentId) {
        setCodingAgent(null);
      }

      // Reload policy agent if changed
      const prevServiceAgentId = resolveServiceAgentId(portal);
      if (effectiveServiceAgentId && effectiveServiceAgentId !== prevServiceAgentId) {
        try {
          const a = await agentApi.getAgent(effectiveServiceAgentId);
          setPolicyAgent(a);
        } catch { setPolicyAgent(null); }
      } else if (!effectiveServiceAgentId) {
        setPolicyAgent(null);
      }

      // Only force new session when the coding agent actually changed.
      // Other settings (prompt, docs, extensions) are synced by the backend.
      if (effectiveCodingAgentId !== prevCodingAgentId) {
        clearPersistedSession();
      }
      addToast('success', t('ecosystem.saveSuccess'));
    } catch {
      addToast('error', t('ecosystem.operationError'));
    } finally {
      setSavingSettings(false);
    }
  };

  const toggleDocId = (docId: string) => setSelectedDocIds(prev => toggleItem(prev, docId));
  const toggleExtension = (ext: string) => setSelectedExtensions(prev => toggleItem(prev, ext));
  const toggleSkillId = (skillId: string) => setSelectedSkillIds(prev => toggleItem(prev, skillId));

  if (loading || !portal) {
    return <LoadingState className="py-24" />;
  }

  const portalStatusVariant = portal.status === 'published' ? 'success' as const : 'warning' as const;

  const codingAgentId = resolveCodingAgentId(portal);
  const previewIndexUrl = portal.status === 'published'
    ? `/p/${portal.slug}/`
    : portal.previewUrl;
  const previewBaseUrl = previewIndexUrl.replace(/\/$/, '');
  const previewRouteLabel = portal.status === 'published' ? `/p/${portal.slug}` : '内部预览';
  const canPreviewViaIframe = !!portal.projectPath && !!previewIndexUrl;
  const extensionOptions = policyAgent ? getRuntimeExtensionOptions(policyAgent) : [];
  const skillOptions = policyAgent
    ? (policyAgent.assigned_skills || []).filter(s => s.enabled)
    : [];
  const effectiveServiceSourceId = editServiceAgentId || editCodingAgentId || null;
  const effectiveServiceSourceAgent = findAgentById(effectiveServiceSourceId);
  const currentServiceBindingMode = classifyPortalServiceAgent(effectiveServiceSourceAgent);
  const linkedAvatar =
    avatarInstances.find(item => item.serviceAgentId === effectiveServiceSourceAgent?.id) || null;
  const serviceAgentGroups = groupPortalServiceAgents(agents);
  const timelineEvents = [...runtimeEvents].reverse();
  const selectedFileUrl = selectedFilePath
    ? `${previewBaseUrl}/${selectedFilePath.split('/').map(s => encodeURIComponent(s)).join('/')}`
    : '';

  const runtimeBadgeClass = (kind: ChatRuntimeEvent['kind']): string => {
    switch (kind) {
      case 'toolcall':
      case 'toolresult':
        return 'bg-status-info-text/75';
      case 'workspace_changed':
        return 'bg-status-success-text/75';
      case 'compaction':
        return 'bg-status-warning-text/75';
      case 'goal':
        return 'bg-status-error-text/75';
      case 'done':
        return 'bg-[hsl(var(--ui-line-strong))/0.62]';
      case 'connection':
        return 'bg-primary/75';
      default:
        return 'bg-primary';
    }
  };

  const deviceWidthMap: Record<PreviewDevice, string> = { desktop: '100%', tablet: '768px', mobile: '375px' };
  const deviceWidthStyle = deviceWidthMap[previewDevice];
  const serviceBindingModeBadge =
    currentServiceBindingMode === 'shared_avatar'
      ? {
          label: t('ecosystem.serviceBindingModeBadgeShared', '共享分身服务'),
          className: 'border-[hsl(var(--status-warning-text))/0.18] bg-status-warning-bg text-status-warning-text',
        }
      : currentServiceBindingMode === 'direct_ecosystem'
        ? {
            label: t('ecosystem.serviceBindingModeBadgeDedicated', '生态专用服务'),
            className: 'border-[hsl(var(--status-success-text))/0.18] bg-status-success-bg text-status-success-text',
          }
        : currentServiceBindingMode === 'clone_general'
          ? {
              label: t('ecosystem.serviceBindingModeBadgeCloneOnSave', '保存后复制为专用服务'),
              className: 'border-[hsl(var(--status-info-text))/0.18] bg-status-info-bg text-status-info-text',
            }
          : null;

  return (
    <div className="flex flex-col h-[calc(100vh-40px)] overflow-hidden">
      {/* Compact header */}
      <div className="border-b shrink-0 bg-background/95 backdrop-blur-sm min-w-0">
        <div className="flex items-start gap-2 px-3 py-2 min-w-0">
          <Button variant="ghost" size="sm" onClick={onBack} className="h-7 w-7 p-0 shrink-0 mt-0.5">
            <ArrowLeft className="w-3.5 h-3.5" />
          </Button>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2 min-w-0">
              <h2 className="text-[13px] font-semibold truncate">{portal.name}</h2>
              <StatusBadge status={portalStatusVariant} className="shrink-0">
                {t(`ecosystem.status.${portal.status}`)}
              </StatusBadge>
              {serviceBindingModeBadge && (
                <span className={`hidden sm:inline-flex shrink-0 items-center rounded-full border px-2 py-0.5 text-[11px] font-medium ${serviceBindingModeBadge.className}`}>
                  {serviceBindingModeBadge.label}
                </span>
              )}
            </div>
            {!isMobile && (
              <div className="mt-1 flex items-center gap-1.5 text-caption text-muted-foreground min-w-0">
                <span className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-muted/20 px-2 py-0.5 min-w-0">
                  <Globe className="w-3 h-3 shrink-0" />
                  <span className="truncate">{previewRouteLabel}</span>
                </span>
                <button onClick={copyUrl} className="p-1 rounded hover:text-foreground hover:bg-muted/40" title={t('ecosystem.copyUrl')}>
                  {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                </button>
                {portal.publicUrl && portal.testPublicUrl && portal.publicUrl !== portal.testPublicUrl && (
                  <button onClick={copyTestUrl} className="px-1.5 py-0.5 rounded border border-border/60 hover:text-foreground hover:bg-muted/40" title={t('ecosystem.copyTestUrl', 'Copy test URL (IP:port)')}>
                    {copiedTest ? <Check className="w-3 h-3" /> : 'IP'}
                  </button>
                )}
              </div>
            )}
          </div>
          <div className="ml-auto flex items-center gap-1 shrink-0">
            {canManage && (
              <>
                <Button size="sm" variant={portal.status === 'published' ? 'outline' : 'default'} onClick={handlePublish} disabled={publishLoading} className="h-7 text-caption px-2.5">
                  {publishLoading && <Loader2 className="w-3 h-3 animate-spin mr-1" />}
                  {portal.status === 'published' ? t('ecosystem.unpublish') : t('ecosystem.publish')}
                </Button>
                {!isMobile && (
                  <button onClick={() => setShowSettings(s => !s)} className={`p-1.5 rounded-md transition-colors ${showSettings ? 'bg-muted text-primary' : 'text-muted-foreground hover:bg-muted'}`} title={t('ecosystem.settings')}>
                    <Settings className="w-3.5 h-3.5" />
                  </button>
                )}
                <button onClick={() => setShowDeleteConfirm(true)} className="p-1.5 rounded-md text-muted-foreground hover:bg-muted">
                  <Trash2 className="w-3.5 h-3.5 text-[hsl(var(--destructive))]" />
                </button>
              </>
            )}
          </div>
        </div>
        {isMobile && (
          <div className="px-3 pb-2 flex items-center gap-1.5 text-caption text-muted-foreground">
            <span className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-muted/20 px-2 py-0.5 max-w-full">
              <Globe className="w-3 h-3 shrink-0" />
              <span className="truncate">{previewRouteLabel}</span>
            </span>
            <button onClick={copyUrl} className="p-1 rounded hover:text-foreground hover:bg-muted/40" title={t('ecosystem.copyUrl')}>
              {copied ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
            </button>
            {portal.publicUrl && portal.testPublicUrl && portal.publicUrl !== portal.testPublicUrl && (
              <button onClick={copyTestUrl} className="px-1.5 py-0.5 rounded border border-border/60 hover:text-foreground hover:bg-muted/40" title={t('ecosystem.copyTestUrl', 'Copy test URL (IP:port)')}>
                {copiedTest ? <Check className="w-3 h-3" /> : 'IP'}
              </button>
            )}
          </div>
        )}
      </div>

      {/* Mobile tab bar */}
      {isMobile && (
        <div className="flex border-b shrink-0 bg-background">
          {([
            ['chat', MessageCircle, t('chat.title', 'Chat')],
            ['preview', Eye, t('ecosystem.preview', 'Preview')],
            ['settings', Settings, t('ecosystem.settings')],
          ] as const).map(([key, Icon, label]) => (
            <button
              key={key}
              onClick={() => setMobileTab(key)}
              className={`flex-1 flex items-center justify-center gap-1.5 py-2 text-xs font-medium transition-colors ${
                mobileTab === key
                  ? 'text-primary border-b-2 border-primary'
                  : 'text-muted-foreground hover:text-foreground'
              }`}
            >
              <Icon className="w-3.5 h-3.5" />
              {label}
            </button>
          ))}
        </div>
      )}

      {/* Main: chat + preview + settings drawer */}
      <div className="flex flex-1 min-h-0 min-w-0 overflow-hidden">
        {/* Chat panel */}
        <div className={`${isMobile ? (mobileTab === 'chat' ? 'flex-1' : 'hidden') : 'md:w-[min(36vw,400px)] border-r shrink-0'} flex flex-col`}>
          {codingAgent && codingAgentId ? (
            <ChatConversation
              sessionId={chatSessionId}
              agentId={codingAgentId}
              agentName={codingAgent.name}
              agent={codingAgent}
              headerVariant="compact"
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
              <p className="text-sm text-center">{t('ecosystem.noCodingAgentSelected', 'Please select a coding agent first')}</p>
              <Button size="sm" variant="outline" onClick={() => { if (isMobile) setMobileTab('settings'); else setShowSettings(true); }}>
                {t('ecosystem.codingAgentSelect', 'Coding Agent')}
              </Button>
            </div>
          )}
        </div>

        {/* Preview area */}
        <div className={`${isMobile ? (mobileTab === 'preview' ? 'flex-1' : 'hidden') : 'flex-1'} flex flex-col min-w-0 relative`}>
          {/* Preview toolbar */}
          <div className="flex items-center gap-1 px-2 py-1 border-b border-border/40 shrink-0 text-xs">
            {!isMobile && (
              <div className="flex items-center gap-0.5 bg-muted/50 rounded-md p-0.5">
                {([['desktop', Monitor], ['tablet', Tablet], ['mobile', Smartphone]] as const).map(([key, Icon]) => (
                  <button key={key} onClick={() => setPreviewDevice(key)} className={`p-1 rounded ${previewDevice === key ? 'bg-background shadow-sm text-foreground' : 'text-muted-foreground hover:text-foreground'}`}>
                    <Icon className="w-3.5 h-3.5" />
                  </button>
                ))}
              </div>
            )}
            <button className="p-1 text-muted-foreground hover:text-foreground" onClick={() => setPreviewKey(k => k + 1)} title={t('ecosystem.refreshPreview')}>
              <RefreshCw className="w-3.5 h-3.5" />
            </button>
            {chatProcessing && <span className="text-caption text-muted-foreground ml-1 animate-pulse">{t('chat.processing', 'Processing...')}</span>}
            <div className="ml-auto flex items-center gap-0.5">
              {([
                ['files', FolderTree, t('ecosystem.files', 'Files')],
                ['activity', Activity, t('ecosystem.activity', 'Activity')],
                ['analytics', BarChart3, t('ecosystem.analytics')],
              ] as const).map(([key, Icon, label]) => (
                <button key={key} onClick={() => setActivePanel(prev => prev === key ? null : key)} className={`flex items-center gap-1 px-2 py-1 rounded-md transition-colors ${activePanel === key ? 'bg-primary/10 text-primary' : 'text-muted-foreground hover:text-foreground hover:bg-muted/50'}`}>
                  <Icon className="w-3.5 h-3.5" />
                  <span className="hidden sm:inline">{label}</span>
                </button>
              ))}
            </div>
          </div>

          {/* Preview iframe + floating overlay */}
          <div className="flex-1 relative overflow-hidden bg-muted/20">
            {canPreviewViaIframe ? (
              <div className="h-full flex items-start justify-center overflow-auto" style={!isMobile && previewDevice !== 'desktop' ? { padding: '12px' } : undefined}>
                <iframe
                  ref={iframeRef}
                  key={previewKey}
                  src={previewIndexUrl}
                  className="border-0 bg-[hsl(var(--ui-surface-panel))/0.96]"
                  style={{
                    width: isMobile ? '100%' : deviceWidthStyle,
                    height: '100%',
                    maxWidth: '100%',
                    ...(!isMobile && previewDevice !== 'desktop' ? { borderRadius: '12px', border: '1px solid hsl(var(--ui-line-soft) / 0.72)' } : {}),
                  }}
                  title="Portal Preview"
                />
              </div>
            ) : (
              <div className="flex items-center justify-center h-full text-muted-foreground text-sm">
                {t('ecosystem.agentHint')}
              </div>
            )}

            {/* Floating overlay panel */}
            {activePanel === 'files' && (
              <div className="absolute inset-0 bg-background/98 backdrop-blur-sm flex flex-col z-10">
                <div className="px-3 py-2 border-b border-border/40 flex items-center gap-2 text-xs">
                  <button className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-50" onClick={() => fileParentPath != null && loadFiles(fileParentPath)} disabled={loadingFiles || fileParentPath == null} title={t('ecosystem.parentDir', 'Parent Directory')}>
                    <ChevronUp className="w-3.5 h-3.5" />
                  </button>
                  <button className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-50" onClick={() => loadFiles(filePath || '')} disabled={loadingFiles} title={t('ecosystem.refreshFiles', 'Refresh files')}>
                    <RefreshCw className={`w-3.5 h-3.5 ${loadingFiles ? 'animate-spin' : ''}`} />
                  </button>
                  <span className="font-mono text-muted-foreground truncate">/{filePath || ''}</span>
                  {chatProcessing && (
                    <span className="ml-auto text-caption text-muted-foreground">{t('ecosystem.autoRefreshing', 'Auto refreshing while Agent is running')}</span>
                  )}
                  <button onClick={() => setActivePanel(null)} className="ml-auto p-1 rounded hover:bg-muted text-muted-foreground"><X className="w-3.5 h-3.5" /></button>
                </div>
                <div className="flex-1 min-h-0">
                  {!portal.projectPath ? (
                    <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                      {t('ecosystem.noProjectPath', 'Project path not initialized')}
                    </div>
                  ) : (
                    <div className="h-full grid grid-cols-1 md:grid-cols-[minmax(280px,320px)_1fr]">
                      <div className="border-b border-border/40 md:border-b-0 md:border-r md:border-border/40 overflow-auto">
                        {fileError ? (
                          <div className="p-4 text-sm text-[hsl(var(--destructive))]">{fileError}</div>
                        ) : loadingFiles && fileEntries.length === 0 ? (
                          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                            {t('common.loading')}
                          </div>
                        ) : fileEntries.length === 0 ? (
                          <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                            {t('ecosystem.emptyFolder', 'Folder is empty')}
                          </div>
                        ) : (
                          <div className="divide-y divide-border/15">
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
                                  <Folder className="w-4 h-4 shrink-0 text-status-info-text" />
                                ) : (
                                  <FileText className="w-4 h-4 text-muted-foreground shrink-0" />
                                )}
                                <div className="min-w-0 flex-1">
                                  <div className="text-sm truncate">{entry.name}</div>
                                  <div className="text-caption text-muted-foreground truncate">
                                    {entry.path}
                                  </div>
                                </div>
                                {!entry.isDir && (
                                  <div className="text-caption text-muted-foreground shrink-0 text-right">
                                    <div>{formatBytes(entry.size)}</div>
                                    {entry.modifiedAt && (
                                      <div>{formatTime(entry.modifiedAt)}</div>
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
                            {t('ecosystem.selectFileToPreview', 'Select a file to preview its content')}
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
                            <div className="px-3 py-2 border-b border-border/40 flex items-start gap-2">
                              <div className="min-w-0 flex-1">
                                <div className="text-sm font-medium truncate">{selectedFile.name}</div>
                                <div className="text-caption text-muted-foreground font-mono truncate">
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
                                  className="text-caption px-2 py-1 border rounded hover:bg-muted whitespace-nowrap"
                                >
                                  {t('documents.openInNewTab', 'Open in new tab')}
                                </a>
                              )}
                            </div>
                            <div className="px-3 py-2 border-b border-border/15 text-caption text-muted-foreground flex flex-wrap gap-x-3 gap-y-1">
                              <span>{selectedFile.contentType}</span>
                              <span>{formatBytes(selectedFile.size)}</span>
                              {selectedFile.modifiedAt && (
                                <span>{formatDateTime(selectedFile.modifiedAt)}</span>
                              )}
                              {selectedFile.truncated && (
                                <span className="text-status-warning-text">
                                  {t('ecosystem.filePreviewTruncated', 'Preview truncated to first 512 KB')}
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
                                  {t('ecosystem.binaryPreviewUnavailable', 'Binary file preview is not available')}
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

            {activePanel === 'activity' && (
              <div className="absolute inset-0 bg-background/98 backdrop-blur-sm flex flex-col z-10">
                <div className="px-3 py-2 border-b border-border/40 flex items-center gap-2 text-xs">
                  <span className="font-medium">{t('ecosystem.activity', 'Activity')}</span>
                  <span className="text-muted-foreground">({runtimeEvents.length})</span>
                  {chatProcessing && (
                    <span className="ml-2 text-muted-foreground">{t('ecosystem.activityLive', 'Live updates')}</span>
                  )}
                  <button className="ml-auto px-2 py-1 border rounded hover:bg-muted" onClick={() => {
                    setRuntimeEvents([]);
                    if (chatSessionId) {
                      try { window.localStorage.removeItem(`${runtimeEventsStoragePrefix}${chatSessionId}`); } catch {}
                    }
                  }}>{t('common.reset', 'Reset')}</button>
                  <button onClick={() => setActivePanel(null)} className="p-1 rounded hover:bg-muted text-muted-foreground"><X className="w-3.5 h-3.5" /></button>
                </div>
                <div className="flex-1 overflow-auto">
                  {timelineEvents.length === 0 ? (
                    <div className="h-full flex items-center justify-center text-sm text-muted-foreground">
                      {t('ecosystem.activityEmpty', 'No runtime events yet')}
                    </div>
                  ) : (
                    <div className="divide-y divide-border/15">
                      {timelineEvents.map(item => (
                        <div key={item.id} className="px-3 py-2 flex items-start gap-2">
                          <span className={`mt-1 h-2.5 w-2.5 rounded-full ${runtimeBadgeClass(item.kind)}`} />
                          <div className="min-w-0 flex-1">
                            <div className="text-sm break-words">{item.text}</div>
                            <div className="text-caption text-muted-foreground">
                              {formatTime(new Date(item.ts))}
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            )}

            {activePanel === 'analytics' && (
              <div className="absolute inset-0 bg-background/98 backdrop-blur-sm flex flex-col z-10">
                <div className="px-3 py-2 border-b border-border/40 flex items-center justify-between text-xs">
                  <span className="font-medium">{t('ecosystem.analytics')}</span>
                  <button onClick={() => setActivePanel(null)} className="p-1 rounded hover:bg-muted text-muted-foreground"><X className="w-3.5 h-3.5" /></button>
                </div>
                <div className="p-4 grid gap-4 sm:grid-cols-2">
                  {[
                    { label: t('ecosystem.visitors'), value: stats?.uniqueVisitors ?? '—' },
                    { label: t('ecosystem.pageViews'), value: stats?.pageViews ?? '—' },
                    { label: t('ecosystem.chatMessages'), value: stats?.chatMessages ?? '—' },
                    { label: t('ecosystem.interactions'), value: stats?.totalInteractions ?? '—' },
                  ].map((item) => (
                    <div key={item.label} className="border rounded-lg p-4">
                      <p className="text-sm text-muted-foreground">{item.label}</p>
                      <p className="text-2xl font-semibold mt-1">{item.value}</p>
                    </div>
                  ))}
                </div>
              </div>
            )}

          </div>
        </div>

        {/* Settings drawer */}
        {(isMobile ? mobileTab === 'settings' : showSettings) && (
          <div className={`${isMobile ? 'flex-1' : 'md:w-[min(34vw,380px)] border-l shrink-0'} flex flex-col bg-background min-w-0`}>
            {!isMobile && (
              <div className="px-4 py-2 border-b border-border/40 flex items-center justify-between shrink-0">
                <span className="text-sm font-semibold">{t('ecosystem.settings')}</span>
                <button onClick={() => setShowSettings(false)} className="p-1 rounded hover:bg-muted text-muted-foreground"><X className="w-4 h-4" /></button>
              </div>
            )}
            <div className="flex-1 overflow-y-auto p-4 space-y-5">
              {/* Group 1: Agent Config */}
              <div className="rounded-lg bg-muted/30 p-3 space-y-3">
                <div className="flex items-center gap-1.5 text-caption font-medium text-muted-foreground uppercase tracking-wide">
                  <Bot className="w-3.5 h-3.5" />
                  <span>Agent</span>
                </div>
                <div>
                  <label className="text-xs font-medium">{t('ecosystem.codingAgentSelect', 'Coding Agent')}</label>
                  <select className="mt-1 w-full rounded-md border bg-background px-2.5 py-1.5 text-sm" value={editCodingAgentId || ''} onChange={(e) => setEditCodingAgentId(e.target.value || null)}>
                    <option value="">{t('ecosystem.noAgentSelected')}</option>
                    {agents.map(a => (<option key={a.id} value={a.id}>{a.name}{a.model ? ` (${a.model})` : ''}</option>))}
                  </select>
                  <p className="text-caption text-muted-foreground mt-0.5">{t('ecosystem.codingAgentHint')}</p>
                </div>
                <div>
                  <label className="text-xs font-medium">{t('ecosystem.serviceAgentSelect', 'Service Agent')}</label>
                  <select className="mt-1 w-full rounded-md border bg-background px-2.5 py-1.5 text-sm" value={editServiceAgentId || ''} onChange={(e) => setEditServiceAgentId(e.target.value || null)}>
                    <option value="">{t('ecosystem.followCodingAgent', 'Follow coding agent')}</option>
                    {serviceAgentGroups.general.length > 0 && (
                      <optgroup label={t('ecosystem.serviceAgentGroupGeneral', '通用模板（将复制）')}>
                        {serviceAgentGroups.general.map(agent => (
                          <option key={agent.id} value={agent.id}>
                            {formatPortalServiceAgentOptionLabel(t, agent)}
                          </option>
                        ))}
                      </optgroup>
                    )}
                    {serviceAgentGroups.ecosystem.length > 0 && (
                      <optgroup label={t('ecosystem.serviceAgentGroupEcosystem', '生态专用服务（直接接入）')}>
                        {serviceAgentGroups.ecosystem.map(agent => (
                          <option key={agent.id} value={agent.id}>
                            {formatPortalServiceAgentOptionLabel(t, agent)}
                          </option>
                        ))}
                      </optgroup>
                    )}
                    {serviceAgentGroups.avatar.length > 0 && (
                      <optgroup label={t('ecosystem.serviceAgentGroupAvatar', '数字分身服务（共享接入）')}>
                        {serviceAgentGroups.avatar.map(agent => (
                          <option key={agent.id} value={agent.id}>
                            {formatPortalServiceAgentOptionLabel(t, agent)}
                          </option>
                        ))}
                      </optgroup>
                    )}
                    {serviceAgentGroups.blocked.length > 0 && (
                      <optgroup label={t('ecosystem.serviceAgentGroupBlocked', '不可用于服务')}>
                        {serviceAgentGroups.blocked.map(agent => (
                          <option key={agent.id} value={agent.id} disabled>
                            {formatPortalServiceAgentOptionLabel(t, agent)}
                          </option>
                        ))}
                      </optgroup>
                    )}
                  </select>
                  <p className="text-caption text-muted-foreground mt-0.5">
                    {describePortalServiceBindingMode(t, currentServiceBindingMode)}
                  </p>
                  {(currentServiceBindingMode === 'shared_avatar' ||
                    currentServiceBindingMode === 'direct_ecosystem' ||
                    currentServiceBindingMode === 'clone_general') && (
                    <div className="mt-2 rounded-md border border-border/60 bg-background px-2.5 py-2.5 text-caption">
                      <div className="flex flex-wrap items-start justify-between gap-2">
                        <div>
                          <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                            {t('ecosystem.serviceBindingPanelTitle', '服务治理归属')}
                          </p>
                          <div className="mt-1 flex flex-wrap items-center gap-2">
                            <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium ${
                              currentServiceBindingMode === 'shared_avatar'
                                ? 'border-[hsl(var(--status-warning-text))/0.18] bg-status-warning-bg text-status-warning-text'
                                : currentServiceBindingMode === 'direct_ecosystem'
                                  ? 'border-[hsl(var(--status-success-text))/0.18] bg-status-success-bg text-status-success-text'
                                  : 'border-[hsl(var(--status-info-text))/0.18] bg-status-info-bg text-status-info-text'
                            }`}>
                              {currentServiceBindingMode === 'shared_avatar'
                                ? t('ecosystem.serviceBindingPanelSharedTitle', '共享数字分身服务')
                                : currentServiceBindingMode === 'direct_ecosystem'
                                  ? t('ecosystem.serviceBindingPanelDedicatedTitle', '生态专用服务')
                                  : t('ecosystem.serviceBindingPanelCloneTitle', '保存后复制为生态专用服务')}
                            </span>
                            {effectiveServiceSourceAgent && (
                              <span className="text-foreground/90">
                                {t('ecosystem.serviceBindingPanelServiceAgent', '服务 Agent')}:
                                <span className="ml-1 font-medium">{effectiveServiceSourceAgent.name}</span>
                              </span>
                            )}
                            {linkedAvatar && (
                              <span className="text-foreground/90">
                                {t('ecosystem.serviceBindingPanelAvatar', '关联分身')}:
                                <span className="ml-1 font-medium">{linkedAvatar.name}</span>
                              </span>
                            )}
                          </div>
                        </div>
                        {currentServiceBindingMode === 'shared_avatar' && (
                          <div className="flex flex-wrap items-center gap-1.5">
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              className="h-7 px-2 text-caption"
                              onClick={() => navigate(`/teams/${teamId}?section=digital-avatar`)}
                            >
                              {t('ecosystem.serviceBindingPanelOpenAvatarWorkspace', '打开数字分身工作台')}
                            </Button>
                            {linkedAvatar && (
                              <Button
                                type="button"
                                size="sm"
                                variant="outline"
                                className="h-7 px-2 text-caption"
                                onClick={() => navigate(`/teams/${teamId}/digital-avatars/${linkedAvatar.portalId}/timeline`)}
                              >
                                {t('ecosystem.serviceBindingPanelOpenAvatarTimeline', '打开分身治理时间线')}
                              </Button>
                            )}
                          </div>
                        )}
                      </div>
                      <div className="mt-2 grid gap-2 sm:grid-cols-2">
                        <div className="rounded-md border border-border/40 bg-muted/20 px-2 py-2">
                          <p className="font-medium text-foreground/90">
                            {t('ecosystem.serviceBindingPanelCanEditHere', '当前可在这里调整')}
                          </p>
                          <p className="mt-1 text-muted-foreground">
                            {t(
                              'ecosystem.serviceBindingPanelCanEditHereItems',
                              'Portal 对外入口、访客欢迎语、文档绑定范围、Portal 级扩展/技能 allowlist，以及页面与 SDK 接入方式。'
                            )}
                          </p>
                        </div>
                        <div className="rounded-md border border-border/40 bg-muted/20 px-2 py-2">
                          <p className="font-medium text-foreground/90">
                            {t('ecosystem.serviceBindingPanelGovernElsewhere', '底层能力治理位置')}
                          </p>
                          <p className="mt-1 text-muted-foreground">
                            {currentServiceBindingMode === 'shared_avatar'
                              ? t(
                                  'ecosystem.serviceBindingPanelGovernElsewhereShared',
                                  '底层服务 Agent 的提示词、技能、扩展与岗位治理仍归数字分身频道管理。'
                                )
                              : currentServiceBindingMode === 'direct_ecosystem'
                                ? t(
                                    'ecosystem.serviceBindingPanelGovernElsewhereDedicated',
                                    '当前是生态协作专用服务 Agent，适合继续在生态协作中维护入口与对外体验。'
                                  )
                                : t(
                                    'ecosystem.serviceBindingPanelGovernElsewhereClone',
                                    '保存后会先复制出生态协作专用服务 Agent，再由当前 Portal 继续承接对外入口配置。'
                                  )}
                          </p>
                        </div>
                      </div>
                      {currentServiceBindingMode === 'shared_avatar' && !linkedAvatar && (
                        <p className="mt-2 text-[11px] text-muted-foreground">
                          {t(
                            'ecosystem.serviceBindingPanelNoAvatarLinked',
                            '尚未在数字分身目录里定位到该共享服务的对应分身记录，但治理主权仍归数字分身频道。'
                          )}
                        </p>
                      )}
                    </div>
                  )}
                </div>
                <div className="rounded-md border border-border/40 bg-background px-2.5 py-2">
                  <label className="flex items-start gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      className="mt-0.5"
                      checked={editShowChatWidget}
                      onChange={(e) => setEditShowChatWidget(e.target.checked)}
                    />
                    <span>
                      <span className="text-xs font-medium block">
                        {t('ecosystem.showDefaultChatWidget', 'Show default chat widget')}
                      </span>
                      <span className="text-caption text-muted-foreground block mt-0.5">
                        {t(
                          'ecosystem.showDefaultChatWidgetHint',
                          'Turn off if your page already has a custom chat UI to avoid duplicate chat entrances.'
                        )}
                      </span>
                    </span>
                  </label>
                </div>
              </div>

              {/* Group 2: Prompts */}
              <div className="rounded-lg bg-muted/30 p-3 space-y-3">
                <div className="flex items-center gap-1.5 text-caption font-medium text-muted-foreground uppercase tracking-wide">
                  <MessageSquare className="w-3.5 h-3.5" />
                  <span>{t('ecosystem.agentSystemPrompt')}</span>
                </div>
                <textarea className="w-full rounded-md border bg-background px-2.5 py-1.5 text-sm min-h-[68px] resize-y" value={editAgentPrompt} onChange={(e) => setEditAgentPrompt(e.target.value)} placeholder="Optional system prompt override..." />
                <div>
                  <label className="text-xs font-medium">{t('ecosystem.agentWelcomeMessage')}</label>
                  <textarea className="mt-1 w-full rounded-md border bg-background px-2.5 py-1.5 text-sm min-h-[52px] resize-y" value={editWelcomeMsg} onChange={(e) => setEditWelcomeMsg(e.target.value)} placeholder="Welcome message..." />
                </div>
              </div>

              {/* Group 3: Resources & Permissions */}
              <div className="rounded-lg bg-muted/30 p-3 space-y-3">
                <div className="flex items-center gap-1.5 text-caption font-medium text-muted-foreground uppercase tracking-wide">
                  <Shield className="w-3.5 h-3.5" />
                  <span>{t('ecosystem.resourcesAndPermissions')}</span>
                </div>

                {/* Documents */}
                <div>
                  <div className="flex items-center justify-between mb-1.5">
                    <label className="text-xs font-medium">{t('ecosystem.boundDocs')}</label>
                    <button onClick={() => setShowDocPickerSettings(true)} className="flex items-center gap-0.5 text-caption text-primary hover:text-primary/80">
                      <Plus className="w-3 h-3" />{t('common.edit')}
                    </button>
                  </div>
                  {selectedDocIds.length === 0 ? (
                    <p className="text-caption text-muted-foreground">{t('ecosystem.noDocumentsAvailable')}</p>
                  ) : (
                    <div className="flex flex-wrap gap-1">
                      {selectedDocIds.map(id => {
                        const doc = allDocuments.find(d => d.id === id);
                        return (
                          <span key={id} className="inline-flex items-center gap-1 text-caption bg-background border border-border/40 rounded-md px-1.5 py-0.5 max-w-full">
                            <span className="truncate">{doc ? (doc.display_name || doc.name) : id.slice(0, 12)}</span>
                            <button onClick={() => toggleDocId(id)} className="shrink-0 text-muted-foreground hover:text-foreground"><X className="w-2.5 h-2.5" /></button>
                          </span>
                        );
                      })}
                    </div>
                  )}
                </div>

                {/* Document access mode */}
                <div>
                  <label className="text-xs font-medium">
                    {t('ecosystem.documentAccessMode', 'Document Access Mode')}
                  </label>
                  <select
                    className="mt-1 w-full rounded-md border bg-background px-2.5 py-1.5 text-sm"
                    value={editDocumentAccessMode}
                    onChange={(e) =>
                      setEditDocumentAccessMode(
                        e.target.value as PortalDocumentAccessMode
                      )
                    }
                  >
                    <option value="read_only">
                      {t('ecosystem.documentAccessModeReadOnly', 'Read only')}
                    </option>
                    <option value="co_edit_draft">
                      {t(
                        'ecosystem.documentAccessModeCoEditDraft',
                        'Collaborative draft'
                      )}
                    </option>
                    <option value="controlled_write">
                      {t(
                        'ecosystem.documentAccessModeControlledWrite',
                        'Controlled write'
                      )}
                    </option>
                  </select>
                  <p className="text-caption text-muted-foreground mt-1">
                    {editDocumentAccessMode === 'co_edit_draft'
                      ? t(
                          'ecosystem.documentAccessModeCoEditDraftHint',
                          'Visitors can create/update agent drafts within bound scope.'
                        )
                      : editDocumentAccessMode === 'controlled_write'
                      ? t(
                          'ecosystem.documentAccessModeControlledWriteHint',
                          'Visitors can write with stricter policy controls.'
                        )
                      : t(
                          'ecosystem.documentAccessModeReadOnlyHint',
                          'Visitors can only read/search/list bound documents.'
                        )}
                  </p>
                  <div className="mt-2 rounded-md border border-border/40 bg-background/60 p-2">
                    <div className="text-caption font-medium">
                      {t('ecosystem.effectivePermissionPreview', 'Effective permission preview')}:
                    </div>
                    <ul className="mt-1 space-y-0.5 text-caption text-muted-foreground">
                      <li>
                        {editDocumentAccessMode === 'read_only'
                          ? t(
                              'ecosystem.permissionPreviewRead',
                              'Read/list/search bound documents'
                            )
                          : t(
                              'ecosystem.permissionPreviewReadWrite',
                              'Read/list/search bound documents + create documents'
                            )}
                      </li>
                      <li>
                        {editDocumentAccessMode === 'co_edit_draft'
                          ? t(
                              'ecosystem.permissionPreviewDraftOnly',
                              'Update limited to agent drafts (bound scope/current session)'
                            )
                          : editDocumentAccessMode === 'read_only'
                          ? t(
                              'ecosystem.permissionPreviewNoUpdate',
                              'Document update is disabled'
                            )
                          : t(
                              'ecosystem.permissionPreviewControlledWrite',
                              'Update follows controlled write policy'
                            )}
                      </li>
                      <li>
                        {t('ecosystem.permissionPreviewBoundDocs', 'Bound document scope still applies')}
                      </li>
                    </ul>
                  </div>
                </div>

                {/* Extensions */}
                <div>
                  <div className="flex items-center justify-between mb-1.5">
                    <label className="text-xs font-medium">{t('ecosystem.allowedExtensionsVisitor')}</label>
                    <button onClick={() => setSelectorDialog('extensions')} disabled={!(editServiceAgentId || editCodingAgentId)} className="flex items-center gap-0.5 text-caption text-primary hover:text-primary/80 disabled:text-muted-foreground disabled:cursor-not-allowed">
                      <Plus className="w-3 h-3" />{t('common.edit')}
                    </button>
                  </div>
                  {selectedExtensions.length === 0 ? (
                    <p className="text-caption text-muted-foreground">{!(editServiceAgentId || editCodingAgentId) ? t('ecosystem.selectServiceAgentFirst') : t('ecosystem.noEnabledExtensionsOnAgent')}</p>
                  ) : (
                    <div className="flex flex-wrap gap-1">
                      {selectedExtensions.map(id => {
                        const ext = extensionOptions.find(e => e.id === id);
                        return (
                          <span key={id} className="inline-flex items-center gap-1 text-caption bg-background border border-border/40 rounded-md px-1.5 py-0.5">
                            <span className="max-w-[8rem] truncate sm:max-w-[10rem]">{ext?.label || id}</span>
                            <button onClick={() => toggleExtension(id)} className="text-muted-foreground hover:text-foreground"><X className="w-2.5 h-2.5" /></button>
                          </span>
                        );
                      })}
                    </div>
                  )}
                </div>

                {/* Skills */}
                <div>
                  <div className="flex items-center justify-between mb-1.5">
                    <label className="text-xs font-medium">{t('ecosystem.allowedSkillsVisitor')}</label>
                    <button onClick={() => setSelectorDialog('skills')} disabled={!(editServiceAgentId || editCodingAgentId)} className="flex items-center gap-0.5 text-caption text-primary hover:text-primary/80 disabled:text-muted-foreground disabled:cursor-not-allowed">
                      <Plus className="w-3 h-3" />{t('common.edit')}
                    </button>
                  </div>
                  {selectedSkillIds.length === 0 ? (
                    <p className="text-caption text-muted-foreground">{!(editServiceAgentId || editCodingAgentId) ? t('ecosystem.selectServiceAgentFirst') : t('ecosystem.noAssignedSkillsOnAgent')}</p>
                  ) : (
                    <div className="flex flex-wrap gap-1">
                      {selectedSkillIds.map(id => {
                        const skill = skillOptions.find(s => s.skill_id === id);
                        return (
                          <span key={id} className="inline-flex items-center gap-1 text-caption bg-background border border-border/40 rounded-md px-1.5 py-0.5">
                            <span className="max-w-[8rem] truncate sm:max-w-[10rem]">{skill?.name || id}</span>
                            <button onClick={() => toggleSkillId(id)} className="text-muted-foreground hover:text-foreground"><X className="w-2.5 h-2.5" /></button>
                          </span>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>

              {/* Project path (read-only) */}
              {portal.projectPath && (
                <div className="text-caption text-muted-foreground font-mono bg-muted/30 rounded-lg px-3 py-2 truncate overflow-hidden">
                  {portal.projectPath}
                </div>
              )}

              {canManage && (
                <Button onClick={handleSaveSettings} disabled={savingSettings} className="w-full">
                  {savingSettings && <Loader2 className="w-4 h-4 animate-spin mr-1.5" />}
                  {savingSettings ? t('common.saving') : t('common.save')}
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
      {/* Selector Dialog */}
      <Dialog open={selectorDialog !== null} onOpenChange={(open) => { if (!open) setSelectorDialog(null); }}>
        <DialogContent className="max-w-[92vw] sm:max-w-md max-h-[70vh] flex flex-col">
          <DialogHeader>
            <DialogTitle className="text-sm">
              {selectorDialog === 'extensions' && t('ecosystem.allowedExtensionsVisitor')}
              {selectorDialog === 'skills' && t('ecosystem.allowedSkillsVisitor')}
            </DialogTitle>
            <DialogDescription className="text-xs">
              {selectorDialog === 'extensions' && t('ecosystem.enabledForExternalUsers', { count: selectedExtensions.length })}
              {selectorDialog === 'skills' && t('ecosystem.skillsEnabledForExternalUsers', { count: selectedSkillIds.length })}
            </DialogDescription>
          </DialogHeader>
          <div className="flex-1 overflow-y-auto -mx-6 px-6">
            {selectorDialog === 'extensions' && (
              extensionOptions.length === 0
                ? <p className="text-sm text-muted-foreground py-6 text-center">{t('ecosystem.noEnabledExtensionsOnAgent')}</p>
                : extensionOptions.map(ext => (
                  <label key={ext.id} className="flex items-center gap-2.5 py-2 cursor-pointer border-b border-border/15 last:border-b-0">
                    <input type="checkbox" checked={selectedExtensions.includes(ext.id)} onChange={() => toggleExtension(ext.id)} className="h-4 w-4 rounded border-[hsl(var(--ui-line-strong))/0.72] accent-[hsl(var(--primary))]" />
                    <span className="flex-1 min-w-0">
                      <span className="text-sm block">{ext.label}</span>
                      <span className="text-caption text-muted-foreground block truncate">{ext.description}</span>
                    </span>
                    <span className="text-micro uppercase text-muted-foreground">{ext.source}</span>
                  </label>
                ))
            )}
            {selectorDialog === 'skills' && (
              skillOptions.length === 0
                ? <p className="text-sm text-muted-foreground py-6 text-center">{t('ecosystem.noAssignedSkillsOnAgent')}</p>
                : skillOptions.map(skill => (
                  <label key={skill.skill_id} className="flex items-center gap-2.5 py-2 cursor-pointer border-b border-border/15 last:border-b-0">
                    <input type="checkbox" checked={selectedSkillIds.includes(skill.skill_id)} onChange={() => toggleSkillId(skill.skill_id)} className="h-4 w-4 rounded border-[hsl(var(--ui-line-strong))/0.72] accent-[hsl(var(--primary))]" />
                    <span className="text-sm truncate flex-1">{skill.name}</span>
                  </label>
                ))
            )}
          </div>
        </DialogContent>
      </Dialog>

      <DocumentPicker
        teamId={teamId}
        open={showDocPickerSettings}
        onClose={() => setShowDocPickerSettings(false)}
        onSelect={(docs) => {
          setSelectedDocIds(docs.map(d => d.id));
          setAllDocuments(prev => {
            const map = new Map(prev.map(d => [d.id, d]));
            for (const d of docs) map.set(d.id, d);
            return Array.from(map.values());
          });
        }}
        selectedIds={selectedDocIds}
      />

      <ConfirmDialog
        open={showDeleteConfirm}
        onOpenChange={setShowDeleteConfirm}
        title={t('ecosystem.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDelete}
      />
    </div>
  );
}

