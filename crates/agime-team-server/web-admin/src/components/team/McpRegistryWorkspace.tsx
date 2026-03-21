import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Bot,
  ChevronLeft,
  ChevronRight,
  FolderTree,
  Globe,
  MessageSquareText,
  Link2,
  Search,
  Play,
  Plus,
  Power,
  RefreshCw,
  ShieldCheck,
  Terminal,
  Trash2,
  Wrench,
} from 'lucide-react';
import {
  agentApi,
  BUILTIN_EXTENSIONS,
  type BuiltinExtension,
  type CustomExtensionConfig,
  type TeamAgent,
} from '../../api/agent';
import { apiClient } from '../../api/client';
import { chatApi } from '../../api/chat';
import type { SharedExtension } from '../../api/types';
import { useToast } from '../../contexts/ToastContext';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../ui/card';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { AddExtensionToAgentDialog } from './AddExtensionToAgentDialog';
import { ChatConversation } from '../chat/ChatConversation';
import type { ChatInputComposeRequest, ChatInputQuickActionGroup } from '../chat/ChatInput';
import { fetchVisibleChatAgents } from '../chat/visibleChatAgents';
import { Input } from '../ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { Textarea } from '../ui/textarea';

interface McpRegistryWorkspaceProps {
  teamId: string;
  canManage: boolean;
}

type TransportType = CustomExtensionConfig['type'];

interface InstalledMcpEntry {
  agentId: string;
  agentName: string;
  agentRole?: string;
  agentDomain?: string;
  extension: CustomExtensionConfig;
}

interface TeamMcpResourceEntry {
  extension: SharedExtension;
  attachedAgents: TeamAgent[];
}

interface RuntimeCapabilityItem {
  id: string;
  name: string;
  category: 'platform' | 'builtin_mcp' | 'blocked_legacy';
}

interface InstallFormState {
  attachAgentId: string;
  name: string;
  type: TransportType;
  uriOrCmd: string;
  argsText: string;
  envText: string;
}

interface McpTemplatePreset {
  id: string;
  title: string;
  description: string;
  category: 'filesystem' | 'browser' | 'remote' | 'script';
  type: TransportType;
  name: string;
  uriOrCmd: string;
  argsText: string;
  envText: string;
  note: string;
  keywords: string[];
}

type WorkspaceView = 'chat' | 'manage' | 'advanced';

const DEFAULT_FORM: InstallFormState = {
  attachAgentId: '__none__',
  name: '',
  type: 'stdio',
  uriOrCmd: '',
  argsText: '',
  envText: '',
};

const NO_ATTACH_AGENT = '__none__';
const MCP_EXTENSION_TYPES = new Set(['stdio', 'sse', 'streamable_http', 'streamablehttp']);
const BLOCKED_LEGACY_BUILTINS = new Set<BuiltinExtension>([
  'team',
  'extension_manager',
  'chat_recall',
]);

function parseLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function parseEnvMap(text: string): Record<string, string> {
  const entries = parseLines(text)
    .map((line) => {
      const separator = line.indexOf('=');
      if (separator <= 0) return null;
      const key = line.slice(0, separator).trim();
      const value = line.slice(separator + 1).trim();
      return key ? [key, value] as const : null;
    })
    .filter((item): item is readonly [string, string] => item !== null);
  return Object.fromEntries(entries);
}

function formatRole(role?: string): string | null {
  if (!role) return null;
  if (role === 'manager') return 'Manager';
  if (role === 'service') return 'Service';
  return role;
}

function formatTransport(type: string): string {
  if (type === 'streamable_http' || type === 'streamablehttp') return 'Streamable HTTP';
  if (type === 'sse') return 'SSE';
  return 'STDIO';
}

function isMcpExtensionType(type?: string): boolean {
  return Boolean(type && MCP_EXTENSION_TYPES.has(type.toLowerCase()));
}

function getConfigString(config: Record<string, unknown>, key: string): string | null {
  const value = config[key];
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function getExtensionEntryPoint(extension: SharedExtension): string | null {
  return (
    getConfigString(extension.config, 'uri_or_cmd') ||
    getConfigString(extension.config, 'uriOrCmd') ||
    getConfigString(extension.config, 'command')
  );
}

function getExtensionArgsCount(extension: SharedExtension): number {
  const args = extension.config.args;
  return Array.isArray(args) ? args.length : 0;
}

function getExtensionEnvCount(extension: SharedExtension): number {
  const envs = extension.config.envs;
  return envs && typeof envs === 'object' && !Array.isArray(envs)
    ? Object.keys(envs as Record<string, unknown>).length
    : 0;
}

function isLegacyCustomMcp(entry: InstalledMcpEntry): boolean {
  if (!isMcpExtensionType(entry.extension.type)) return false;
  return !entry.extension.source_extension_id;
}

function pickDefaultDriverAgent(agents: TeamAgent[]): TeamAgent | null {
  if (agents.length === 0) return null;
  const preferred =
    agents.find((agent) => agent.status !== 'error' && agent.status !== 'paused') ?? agents[0];
  return preferred ?? null;
}

function getBuiltinCapabilityMeta(extension: BuiltinExtension) {
  const meta = BUILTIN_EXTENSIONS.find((item) => item.id === extension);
  return {
    name: meta?.name ?? extension,
    isPlatform: meta?.isPlatform ?? false,
    blocked: BLOCKED_LEGACY_BUILTINS.has(extension),
  };
}

export function McpRegistryWorkspace({ teamId, canManage }: McpRegistryWorkspaceProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [driverAgents, setDriverAgents] = useState<TeamAgent[]>([]);
  const [teamExtensions, setTeamExtensions] = useState<SharedExtension[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [form, setForm] = useState<InstallFormState>(DEFAULT_FORM);
  const [activeTemplateId, setActiveTemplateId] = useState<string | null>(null);
  const [templateSearch, setTemplateSearch] = useState('');
  const [templateCategoryFilter, setTemplateCategoryFilter] = useState<
    'all' | McpTemplatePreset['category']
  >('all');
  const [submitting, setSubmitting] = useState(false);
  const [mcpSessionId, setMcpSessionId] = useState<string | null>(null);
  const [driverAgentId, setDriverAgentId] = useState<string>('');
  const [defaultDriverAgentId, setDefaultDriverAgentId] = useState<string>('');
  const [composeRequest, setComposeRequest] = useState<ChatInputComposeRequest | null>(null);
  const [workspaceView, setWorkspaceView] = useState<WorkspaceView>('chat');
  const [search, setSearch] = useState('');
  const [page, setPage] = useState(1);
  const [pendingAction, setPendingAction] = useState<{
    kind: 'toggle' | 'remove';
    agentId: string;
    extensionName: string;
    enabled?: boolean;
  } | null>(null);
  const [attachExtension, setAttachExtension] = useState<SharedExtension | null>(null);

  const loadWorkspaceData = useCallback(async () => {
    try {
      setLoading(true);
      const [agentResponse, extensionResponse, visibleChatAgents, teamSettings] = await Promise.all([
        agentApi.listAgents(teamId, 1, 100),
        apiClient.getExtensions(teamId, { page: 1, limit: 200, sort: 'updated_at' }),
        fetchVisibleChatAgents(teamId),
        apiClient.getTeamSettings(teamId),
      ]);
      const items = agentResponse.items ?? [];
      const extensionItems = (extensionResponse.extensions ?? []).filter((extension) =>
        isMcpExtensionType(extension.extensionType),
      );
      const configuredDefaultAgentId = teamSettings.generalAgent?.defaultAgentId ?? '';
      const validDefaultAgentId = visibleChatAgents.some(
        (agent) => agent.id === configuredDefaultAgentId,
      )
        ? configuredDefaultAgentId
        : '';
      setAgents(items);
      setDriverAgents(visibleChatAgents);
      setDefaultDriverAgentId(validDefaultAgentId);
      setTeamExtensions(extensionItems);
      setError('');
      setDriverAgentId((prev) => {
        if (prev && visibleChatAgents.some((agent) => agent.id === prev)) {
          return prev;
        }
        if (validDefaultAgentId) {
          return validDefaultAgentId;
        }
        return pickDefaultDriverAgent(visibleChatAgents)?.id ?? '';
      });
      setForm((prev) => ({
        ...prev,
        attachAgentId:
          prev.attachAgentId && prev.attachAgentId !== NO_ATTACH_AGENT
            ? prev.attachAgentId
            : NO_ATTACH_AGENT,
      }));
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  }, [t, teamId]);

  useEffect(() => {
    void loadWorkspaceData();
  }, [loadWorkspaceData]);

  useEffect(() => {
    setPage(1);
  }, [search]);

  const templates = useMemo<McpTemplatePreset[]>(
    () => [
      {
        id: 'filesystem',
        title: t('teams.resource.mcpWorkspace.templates.filesystem.title'),
        description: t('teams.resource.mcpWorkspace.templates.filesystem.description'),
        category: 'filesystem',
        type: 'stdio',
        name: 'filesystem',
        uriOrCmd: 'npx',
        argsText: '-y\n@modelcontextprotocol/server-filesystem\n/path/to/allowed/root',
        envText: '',
        note: t('teams.resource.mcpWorkspace.templates.filesystem.note'),
        keywords: ['filesystem', 'files', 'directory', 'stdio'],
      },
      {
        id: 'playwright',
        title: t('teams.resource.mcpWorkspace.templates.playwright.title'),
        description: t('teams.resource.mcpWorkspace.templates.playwright.description'),
        category: 'browser',
        type: 'stdio',
        name: 'playwright',
        uriOrCmd: 'npx',
        argsText: '-y\n@playwright/mcp@latest',
        envText: 'PLAYWRIGHT_HEADLESS=1',
        note: t('teams.resource.mcpWorkspace.templates.playwright.note'),
        keywords: ['playwright', 'browser', 'automation', 'web'],
      },
      {
        id: 'remote-sse',
        title: t('teams.resource.mcpWorkspace.templates.remoteSse.title'),
        description: t('teams.resource.mcpWorkspace.templates.remoteSse.description'),
        category: 'remote',
        type: 'sse',
        name: 'remote-sse',
        uriOrCmd: 'http://127.0.0.1:8931/sse',
        argsText: '',
        envText: '',
        note: t('teams.resource.mcpWorkspace.templates.remoteSse.note'),
        keywords: ['remote', 'sse', 'service', 'endpoint'],
      },
      {
        id: 'remote-http',
        title: t('teams.resource.mcpWorkspace.templates.remoteHttp.title'),
        description: t('teams.resource.mcpWorkspace.templates.remoteHttp.description'),
        category: 'remote',
        type: 'streamable_http',
        name: 'remote-http',
        uriOrCmd: 'http://127.0.0.1:8931/mcp',
        argsText: '',
        envText: '',
        note: t('teams.resource.mcpWorkspace.templates.remoteHttp.note'),
        keywords: ['remote', 'http', 'streamable', 'gateway'],
      },
      {
        id: 'local-script',
        title: t('teams.resource.mcpWorkspace.templates.localScript.title'),
        description: t('teams.resource.mcpWorkspace.templates.localScript.description'),
        category: 'script',
        type: 'stdio',
        name: 'local-script',
        uriOrCmd: 'python',
        argsText: '-m\nyour_mcp_server',
        envText: '',
        note: t('teams.resource.mcpWorkspace.templates.localScript.note'),
        keywords: ['python', 'node', 'script', 'stdio', 'local'],
      },
    ],
    [t],
  );

  const activeTemplate = useMemo(
    () => templates.find((template) => template.id === activeTemplateId) ?? null,
    [activeTemplateId, templates],
  );

  const installedEntries = useMemo<InstalledMcpEntry[]>(() => (
    agents.flatMap((agent) => (agent.custom_extensions ?? []).map((extension) => ({
      agentId: agent.id,
      agentName: agent.name,
      agentRole: agent.agent_role,
      agentDomain: agent.agent_domain,
      extension,
    })))
  ), [agents]);

  const resourceEntries = useMemo<TeamMcpResourceEntry[]>(() => (
    teamExtensions.map((extension) => {
      const attachedAgents = agents.filter((agent) =>
        (agent.custom_extensions ?? []).some((customExtension) => {
          const sourceId = customExtension.source_extension_id?.trim();
          if (sourceId && sourceId === extension.id) return true;
          return customExtension.source === 'team' && customExtension.name === extension.name;
        }),
      );
      return { extension, attachedAgents };
    })
  ), [agents, teamExtensions]);

  const legacyEntries = useMemo(
    () => installedEntries.filter((entry) => isLegacyCustomMcp(entry)),
    [installedEntries],
  );

  const filteredResources = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    if (!keyword) return resourceEntries;
    return resourceEntries.filter((entry) => {
      const haystack = [
        entry.extension.name,
        entry.extension.description,
        entry.extension.extensionType,
        getExtensionEntryPoint(entry.extension),
        ...entry.attachedAgents.map((agent) => agent.name),
      ].join(' ').toLowerCase();
      return haystack.includes(keyword);
    });
  }, [resourceEntries, search]);

  const filteredLegacyEntries = useMemo(() => {
    const keyword = search.trim().toLowerCase();
    if (!keyword) return legacyEntries;
    return legacyEntries.filter((entry) => {
      const haystack = [
        entry.extension.name,
        entry.extension.type,
        entry.extension.uri_or_cmd,
        entry.extension.source,
        entry.agentName,
        entry.agentRole,
        entry.agentDomain,
      ].join(' ').toLowerCase();
      return haystack.includes(keyword);
    });
  }, [legacyEntries, search]);

  const pageSize = 12;
  const totalPages = Math.max(1, Math.ceil(filteredResources.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const pagedResources = filteredResources.slice((currentPage - 1) * pageSize, currentPage * pageSize);

  const attachedResourceCount = resourceEntries.filter((entry) => entry.attachedAgents.length > 0).length;
  const stdioCount = resourceEntries.filter((entry) => entry.extension.extensionType === 'stdio').length;
  const remoteCount = resourceEntries.filter((entry) => entry.extension.extensionType !== 'stdio').length;
  const attachAgent = useMemo(
    () =>
      form.attachAgentId && form.attachAgentId !== NO_ATTACH_AGENT
        ? agents.find((agent) => agent.id === form.attachAgentId) ?? null
        : null,
    [agents, form.attachAgentId],
  );
  const driverAgent = useMemo(
    () => driverAgents.find((agent) => agent.id === driverAgentId) ?? null,
    [driverAgentId, driverAgents],
  );
  const teamExtensionIds = useMemo(
    () => new Set(teamExtensions.map((extension) => extension.id)),
    [teamExtensions],
  );
  const driverEnabledBuiltinCapabilities = useMemo<RuntimeCapabilityItem[]>(() => {
    if (!driverAgent) return [];
    return (driverAgent.enabled_extensions ?? [])
      .filter((extension) => extension.enabled)
      .map((extension) => {
        const meta = getBuiltinCapabilityMeta(extension.extension);
        return {
          id: extension.extension,
          name: meta.name,
          category: meta.blocked
            ? 'blocked_legacy'
            : meta.isPlatform
              ? 'platform'
              : 'builtin_mcp',
        } satisfies RuntimeCapabilityItem;
      });
  }, [driverAgent]);
  const driverBuiltinCapabilities = useMemo(
    () =>
      driverEnabledBuiltinCapabilities.filter(
        (extension) => extension.category !== 'blocked_legacy',
      ),
    [driverEnabledBuiltinCapabilities],
  );
  const driverBlockedLegacyCapabilities = useMemo(
    () =>
      driverEnabledBuiltinCapabilities.filter(
        (extension) => extension.category === 'blocked_legacy',
      ),
    [driverEnabledBuiltinCapabilities],
  );
  const driverAttachedTeamMcps = useMemo(() => {
    if (!driverAgent) return [];
    return (driverAgent.custom_extensions ?? []).filter((extension) => {
      if (!extension.enabled || !isMcpExtensionType(extension.type)) return false;
      const sourceId = extension.source_extension_id?.trim();
      if (sourceId && teamExtensionIds.has(sourceId)) return true;
      return extension.source === 'team';
    });
  }, [driverAgent, teamExtensionIds]);
  const driverAttachedCustomExtensions = useMemo(() => {
    if (!driverAgent) return [];
    return (driverAgent.custom_extensions ?? []).filter((extension) => {
      if (!extension.enabled || !isMcpExtensionType(extension.type)) return false;
      const sourceId = extension.source_extension_id?.trim();
      if (sourceId && teamExtensionIds.has(sourceId)) return false;
      return extension.source !== 'team';
    });
  }, [driverAgent, teamExtensionIds]);
  const driverAttachedMcpCount =
    driverAttachedTeamMcps.length + driverAttachedCustomExtensions.length;
  const currentArgs = useMemo(() => parseLines(form.argsText), [form.argsText]);
  const currentEnvs = useMemo(() => parseEnvMap(form.envText), [form.envText]);
  const templateFilters = useMemo(
    () => [
      { value: 'all' as const, label: t('teams.resource.mcpWorkspace.categories.all') },
      { value: 'filesystem' as const, label: t('teams.resource.mcpWorkspace.categories.filesystem') },
      { value: 'browser' as const, label: t('teams.resource.mcpWorkspace.categories.browser') },
      { value: 'remote' as const, label: t('teams.resource.mcpWorkspace.categories.remote') },
      { value: 'script' as const, label: t('teams.resource.mcpWorkspace.categories.script') },
    ],
    [t],
  );
  const filteredTemplates = useMemo(() => {
    const keyword = templateSearch.trim().toLowerCase();
    return templates.filter((template) => {
      const categoryMatch =
        templateCategoryFilter === 'all' || template.category === templateCategoryFilter;
      if (!categoryMatch) return false;
      if (!keyword) return true;
      return [template.title, template.description, template.name, template.type, template.category, ...template.keywords]
        .join(' ')
        .toLowerCase()
        .includes(keyword);
    });
  }, [templateCategoryFilter, templateSearch, templates]);
  const templateCategoryLabel = useCallback((category: McpTemplatePreset['category']) => {
    return t(`teams.resource.mcpWorkspace.categories.${category}`);
  }, [t]);
  const templateCategoryIcon = useCallback((category: McpTemplatePreset['category']) => {
    if (category === 'filesystem') return <FolderTree className="h-4 w-4" />;
    if (category === 'remote') return <Globe className="h-4 w-4" />;
    if (category === 'browser') return <Link2 className="h-4 w-4" />;
    return <Terminal className="h-4 w-4" />;
  }, []);

  const createComposeRequest = useCallback((text: string, autoSend = false) => {
    setComposeRequest({
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`,
      text,
      autoSend,
    });
  }, []);

  const mcpQuickActionGroups = useMemo<ChatInputQuickActionGroup[]>(
    () => [
      {
        key: 'discover',
        label: t('teams.resource.mcpWorkspace.chatActions.discoverLabel'),
        actions: [
          {
            key: 'discover-playwright',
            label: t('teams.resource.mcpWorkspace.chatActions.discoverPlaywright'),
            description: t('teams.resource.mcpWorkspace.chatActions.discoverPlaywrightHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.discoverPlaywrightPrompt'),
              ),
          },
          {
            key: 'list-installed',
            label: t('teams.resource.mcpWorkspace.chatActions.listInstalled'),
            description: t('teams.resource.mcpWorkspace.chatActions.listInstalledHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.listInstalledPrompt'),
              ),
          },
          {
            key: 'inspect-runtime',
            label: t('teams.resource.mcpWorkspace.chatActions.inspectRuntime'),
            description: t('teams.resource.mcpWorkspace.chatActions.inspectRuntimeHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.inspectRuntimePrompt', {
                  agent: driverAgent?.name ?? t('teams.resource.mcpWorkspace.planUnset'),
                }),
              ),
          },
        ],
      },
      {
        key: 'install',
        label: t('teams.resource.mcpWorkspace.chatActions.installLabel'),
        actions: [
          {
            key: 'install-team',
            label: t('teams.resource.mcpWorkspace.chatActions.installTeam'),
            description: t('teams.resource.mcpWorkspace.chatActions.installTeamHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.installTeamPrompt'),
              ),
          },
          {
            key: 'attach-agent',
            label: t('teams.resource.mcpWorkspace.chatActions.attachAgent'),
            description: t('teams.resource.mcpWorkspace.chatActions.attachAgentHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.attachAgentPrompt'),
              ),
          },
        ],
      },
      {
        key: 'govern',
        label: t('teams.resource.mcpWorkspace.chatActions.governLabel'),
        actions: [
          {
            key: 'update',
            label: t('teams.resource.mcpWorkspace.chatActions.update'),
            description: t('teams.resource.mcpWorkspace.chatActions.updateHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.updatePrompt'),
              ),
          },
          {
            key: 'remove',
            label: t('teams.resource.mcpWorkspace.chatActions.remove'),
            description: t('teams.resource.mcpWorkspace.chatActions.removeHint'),
            onSelect: () =>
              createComposeRequest(
                t('teams.resource.mcpWorkspace.chatActions.removePrompt'),
              ),
          },
        ],
      },
    ],
    [createComposeRequest, t],
  );

  const createMcpConversationSession = useCallback(async () => {
    if (!driverAgent) {
      throw new Error(t('teams.resource.mcpWorkspace.chatDriverMissing'));
    }
    const session = await chatApi.createSession(driverAgent.id, [], {
      extraInstructions: t('teams.resource.mcpWorkspace.chatExtraInstructions'),
    });
    return session.session_id;
  }, [driverAgent, t]);

  useEffect(() => {
    setMcpSessionId(null);
    setComposeRequest(null);
  }, [driverAgentId]);

  function applyTemplate(template: McpTemplatePreset): void {
    setActiveTemplateId(template.id);
    setError('');
    setForm((prev) => ({
      attachAgentId: prev.attachAgentId || NO_ATTACH_AGENT,
      name: template.name,
      type: template.type,
      uriOrCmd: template.uriOrCmd,
      argsText: template.argsText,
      envText: template.envText,
    }));
  }

  function resetTemplate(): void {
    setActiveTemplateId(null);
    setForm((prev) => ({
      ...DEFAULT_FORM,
      attachAgentId: prev.attachAgentId || NO_ATTACH_AGENT,
    }));
  }

  async function handleInstall(): Promise<void> {
    if (!form.name.trim() || !form.uriOrCmd.trim()) {
      setError(t('teams.resource.mcpWorkspace.validationRequired'));
      return;
    }
    setSubmitting(true);
    try {
      const response = await apiClient.createExtension({
        teamId,
        name: form.name.trim(),
        extensionType: form.type,
        config: {
          uri_or_cmd: form.uriOrCmd.trim(),
          args: parseLines(form.argsText),
          envs: parseEnvMap(form.envText),
        },
        description: activeTemplate?.description,
        tags: Array.from(new Set(['mcp', activeTemplate?.category].filter(Boolean) as string[])),
      });
      const created =
        response && typeof response === 'object' && 'extension' in response && response.extension
          ? (response.extension as SharedExtension)
          : (response as unknown as SharedExtension);

      addToast('success', t('teams.resource.mcpWorkspace.installSuccess', { name: form.name.trim() }));

      if (created?.id && form.attachAgentId !== NO_ATTACH_AGENT) {
        await agentApi.addTeamExtension(form.attachAgentId, created.id, teamId);
        addToast(
          'success',
          t('teams.resource.mcpWorkspace.attachSuccess', {
            name: created.name,
            agent: attachAgent?.name ?? '',
          }),
        );
      }

      setForm((prev) => ({
        ...DEFAULT_FORM,
        attachAgentId: prev.attachAgentId,
      }));
      setActiveTemplateId(null);
      await loadWorkspaceData();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSubmitting(false);
    }
  }

  async function handleConfirmAction(): Promise<void> {
    if (!pendingAction) return;
    try {
      if (pendingAction.kind === 'toggle') {
        await agentApi.setCustomExtensionEnabled(
          pendingAction.agentId,
          pendingAction.extensionName,
          teamId,
          Boolean(pendingAction.enabled),
        );
        addToast('success', t('teams.resource.mcpWorkspace.toggleSuccess'));
      } else {
        await agentApi.removeCustomExtension(
          pendingAction.agentId,
          pendingAction.extensionName,
          teamId,
        );
        addToast('success', t('teams.resource.mcpWorkspace.removeSuccess'));
      }
      await loadWorkspaceData();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setPendingAction(null);
    }
  }

  return (
    <>
      <Card className="ui-section-panel">
        <CardHeader>
          <CardTitle className="ui-heading text-[24px]">
            {t('teams.resource.mcpWorkspace.title')}
          </CardTitle>
          <CardDescription className="ui-secondary-text">
            {t('teams.resource.mcpWorkspace.description')}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <section className="grid gap-3 md:grid-cols-3">
            <button
              type="button"
              onClick={() => setWorkspaceView('chat')}
              className={[
                'rounded-[24px] border px-4 py-4 text-left transition-colors',
                workspaceView === 'chat'
                  ? 'border-[hsl(var(--semantic-extension))]/28 bg-[hsl(var(--semantic-extension))]/8'
                  : 'border-[hsl(var(--border))] bg-[hsl(var(--background))] hover:border-[hsl(var(--semantic-extension))]/16',
              ].join(' ')}
            >
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                    {t('teams.resource.mcpWorkspace.viewChat')}
                  </div>
                  <div className="mt-2 text-base font-semibold text-[hsl(var(--foreground))]">
                    {t('teams.resource.mcpWorkspace.chatTitle')}
                  </div>
                </div>
                <Badge variant="outline">{driverAgent ? driverAgent.name : '0'}</Badge>
              </div>
              <p className="mt-3 text-sm leading-6 ui-secondary-text">
                {t('teams.resource.mcpWorkspace.viewChatDescription')}
              </p>
              {defaultDriverAgentId && (
                <p className="mt-2 text-xs ui-tertiary-text">
                  {t('teams.resource.mcpWorkspace.defaultDriverHint', {
                    name:
                      driverAgents.find((agent) => agent.id === defaultDriverAgentId)?.name ??
                      defaultDriverAgentId,
                  })}
                </p>
              )}
            </button>

            <button
              type="button"
              onClick={() => setWorkspaceView('manage')}
              className={[
                'rounded-[24px] border px-4 py-4 text-left transition-colors',
                workspaceView === 'manage'
                  ? 'border-[hsl(var(--semantic-extension))]/28 bg-[hsl(var(--semantic-extension))]/8'
                  : 'border-[hsl(var(--border))] bg-[hsl(var(--background))] hover:border-[hsl(var(--semantic-extension))]/16',
              ].join(' ')}
            >
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                    {t('teams.resource.mcpWorkspace.viewManage')}
                  </div>
                  <div className="mt-2 text-base font-semibold text-[hsl(var(--foreground))]">
                    {t('teams.resource.mcpWorkspace.manageTitle')}
                  </div>
                </div>
                <Badge variant="outline">{resourceEntries.length}</Badge>
              </div>
              <p className="mt-3 text-sm leading-6 ui-secondary-text">
                {t('teams.resource.mcpWorkspace.viewManageDescription')}
              </p>
            </button>

            <button
              type="button"
              onClick={() => setWorkspaceView('advanced')}
              className={[
                'rounded-[24px] border px-4 py-4 text-left transition-colors',
                workspaceView === 'advanced'
                  ? 'border-[hsl(var(--semantic-extension))]/28 bg-[hsl(var(--semantic-extension))]/8'
                  : 'border-[hsl(var(--border))] bg-[hsl(var(--background))] hover:border-[hsl(var(--semantic-extension))]/16',
              ].join(' ')}
            >
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                    {t('teams.resource.mcpWorkspace.viewAdvanced')}
                  </div>
                  <div className="mt-2 text-base font-semibold text-[hsl(var(--foreground))]">
                    {t('teams.resource.mcpWorkspace.advancedHeading')}
                  </div>
                </div>
                <Badge variant="outline">{formatTransport(form.type)}</Badge>
              </div>
              <p className="mt-3 text-sm leading-6 ui-secondary-text">
                {t('teams.resource.mcpWorkspace.viewAdvancedDescription')}
              </p>
            </button>
          </section>

          {workspaceView === 'chat' ? (
            <section
              id="mcp-chat-zone"
              className="grid gap-4 xl:grid-cols-[minmax(0,1.28fr)_360px]"
            >
              <div className="overflow-hidden rounded-[28px] border border-[hsl(var(--semantic-extension))]/18 bg-[hsl(var(--background))]">
                <div className="border-b border-[hsl(var(--border))] px-5 py-4">
                  <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
                    <div className="space-y-2">
                      <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                        {t('teams.resource.mcpWorkspace.chatEyebrow')}
                      </div>
                      <h3 className="ui-heading text-[24px] leading-none">
                        {t('teams.resource.mcpWorkspace.chatTitle')}
                      </h3>
                      <p className="max-w-3xl text-sm leading-6 ui-secondary-text">
                        {t('teams.resource.mcpWorkspace.chatDescription')}
                      </p>
                    </div>
                    <div className="w-full max-w-[300px] space-y-2">
                      <label className="text-xs font-semibold uppercase tracking-[0.14em] ui-tertiary-text">
                        {t('teams.resource.mcpWorkspace.chatDriver')}
                      </label>
                      <Select value={driverAgentId || undefined} onValueChange={setDriverAgentId}>
                        <SelectTrigger>
                          <SelectValue
                            placeholder={t('teams.resource.mcpWorkspace.chatDriverPlaceholder')}
                          />
                        </SelectTrigger>
                        <SelectContent>
                          {driverAgents.map((agent) => (
                            <SelectItem key={agent.id} value={agent.id}>
                              {agent.name}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  </div>
                </div>

                {driverAgent ? (
                  <div className="h-[min(72vh,760px)] min-h-[560px]">
                    <ChatConversation
                      key={`${driverAgent.id}:mcp-workspace`}
                      sessionId={mcpSessionId}
                      agentId={driverAgent.id}
                      agentName={driverAgent.name}
                      agent={driverAgent}
                      teamId={teamId}
                      createSession={createMcpConversationSession}
                      onSessionCreated={setMcpSessionId}
                      composeRequest={composeRequest}
                      inputQuickActionGroups={mcpQuickActionGroups}
                      headerVariant="compact"
                    />
                  </div>
                ) : (
                  <div className="flex min-h-[420px] items-center justify-center px-6 py-10">
                    <div className="max-w-md text-center">
                      <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-[hsl(var(--semantic-extension))]/10 text-[hsl(var(--semantic-extension))]">
                        <MessageSquareText className="h-6 w-6" />
                      </div>
                      <div className="mt-4 text-base font-semibold text-[hsl(var(--foreground))]">
                        {t('teams.resource.mcpWorkspace.chatDriverMissing')}
                      </div>
                      <p className="mt-2 text-sm leading-6 ui-secondary-text">
                        {t('teams.resource.mcpWorkspace.chatDriverMissingDescription')}
                      </p>
                    </div>
                  </div>
                )}
              </div>

              <div className="grid gap-3">
                <div className="rounded-[24px] border border-[hsl(var(--semantic-extension))]/18 bg-[hsl(var(--background))] p-4">
                  <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                    {t('teams.resource.mcpWorkspace.truthTitle')}
                  </div>
                  <p className="mt-2 text-sm leading-6 ui-secondary-text">
                    {t('teams.resource.mcpWorkspace.truthDescription')}
                  </p>
                  <div className="mt-4 space-y-3">
                    <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                      <div className="flex flex-wrap items-center gap-2">
                        <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                          {t('teams.resource.mcpWorkspace.chatDriver')}
                        </div>
                        <Badge variant="outline" className="text-[10px]">
                          {t('teams.resource.mcpWorkspace.managementToolLabel')}
                        </Badge>
                      </div>
                      <div className="mt-2 flex items-center gap-2 text-sm font-medium text-[hsl(var(--foreground))]">
                        <Bot className="h-4 w-4 text-[hsl(var(--semantic-agent))]" />
                        <span>{driverAgent?.name ?? t('teams.resource.mcpWorkspace.planUnset')}</span>
                        {driverAgent?.id === defaultDriverAgentId ? (
                          <Badge variant="outline" className="text-[10px]">
                            {t('teams.resource.mcpWorkspace.defaultDriverBadge')}
                          </Badge>
                        ) : null}
                      </div>
                      <p className="mt-2 text-xs leading-5 ui-secondary-text">
                        {t('teams.resource.mcpWorkspace.managementToolDescription')}
                      </p>
                    </div>
                    <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                      <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                        {t('teams.resource.mcpWorkspace.runtimeModelTitle')}
                      </div>
                      <div className="mt-3 flex flex-wrap gap-2">
                        <Badge variant="outline">
                          {t('teams.resource.mcpWorkspace.runtimeBuiltinLabel', {
                            count: driverBuiltinCapabilities.length,
                          })}
                        </Badge>
                        <Badge variant="outline">
                          {t('teams.resource.mcpWorkspace.runtimeAttachedLabel', {
                            count: driverAttachedMcpCount,
                          })}
                        </Badge>
                        <Badge variant="outline">
                          {t('teams.resource.mcpWorkspace.runtimeLibraryLabel', {
                            count: resourceEntries.length,
                          })}
                        </Badge>
                      </div>
                      <p className="mt-3 text-xs leading-5 ui-secondary-text">
                        {t('teams.resource.mcpWorkspace.runtimeModelDescription')}
                      </p>
                    </div>
                    <div className="grid gap-3 sm:grid-cols-3 xl:grid-cols-1">
                      <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                        <div className="flex items-center gap-2 text-sm font-semibold text-[hsl(var(--foreground))]">
                          <ShieldCheck className="h-4 w-4 text-[hsl(var(--semantic-extension))]" />
                          {t('teams.resource.mcpWorkspace.runtimeBuiltinTitle')}
                        </div>
                        <div className="mt-2 text-[24px] font-semibold text-[hsl(var(--foreground))]">
                          {driverBuiltinCapabilities.length}
                        </div>
                        <p className="mt-2 text-xs leading-5 ui-secondary-text">
                          {t('teams.resource.mcpWorkspace.runtimeBuiltinDescription')}
                        </p>
                      </div>
                      <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                        <div className="flex items-center gap-2 text-sm font-semibold text-[hsl(var(--foreground))]">
                          <Power className="h-4 w-4 text-[hsl(var(--semantic-extension))]" />
                          {t('teams.resource.mcpWorkspace.runtimeAttachedTitle')}
                        </div>
                        <div className="mt-2 text-[24px] font-semibold text-[hsl(var(--foreground))]">
                          {driverAttachedMcpCount}
                        </div>
                        <p className="mt-2 text-xs leading-5 ui-secondary-text">
                          {t('teams.resource.mcpWorkspace.runtimeAttachedDescription', {
                            teamCount: driverAttachedTeamMcps.length,
                            customCount: driverAttachedCustomExtensions.length,
                          })}
                        </p>
                      </div>
                      <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                        <div className="flex items-center gap-2 text-sm font-semibold text-[hsl(var(--foreground))]">
                          <Wrench className="h-4 w-4 text-[hsl(var(--semantic-extension))]" />
                          {t('teams.resource.mcpWorkspace.runtimeLibraryTitle')}
                        </div>
                        <div className="mt-2 text-[24px] font-semibold text-[hsl(var(--foreground))]">
                          {resourceEntries.length}
                        </div>
                        <p className="mt-2 text-xs leading-5 ui-secondary-text">
                          {t('teams.resource.mcpWorkspace.runtimeLibraryDescription')}
                        </p>
                      </div>
                    </div>
                    {driverBlockedLegacyCapabilities.length > 0 ? (
                      <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                        <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                          {t('teams.resource.mcpWorkspace.runtimeBlockedTitle')}
                        </div>
                        <div className="mt-2 flex flex-wrap gap-2">
                          {driverBlockedLegacyCapabilities.map((capability) => (
                            <Badge key={capability.id} variant="outline">
                              {capability.name}
                            </Badge>
                          ))}
                        </div>
                        <p className="mt-2 text-xs leading-5 ui-secondary-text">
                          {t('teams.resource.mcpWorkspace.runtimeBlockedDescription')}
                        </p>
                      </div>
                    ) : null}
                    <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/82 px-3 py-3">
                      <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                        {t('teams.resource.mcpWorkspace.chatSuggestedFlowTitle')}
                      </div>
                      <div className="mt-3 space-y-2 text-sm ui-secondary-text">
                        <div>{t('teams.resource.mcpWorkspace.chatSuggestedFlowSearch')}</div>
                        <div>{t('teams.resource.mcpWorkspace.chatSuggestedFlowInstall')}</div>
                        <div>{t('teams.resource.mcpWorkspace.chatSuggestedFlowAttach')}</div>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </section>
          ) : null}

          {workspaceView === 'advanced' ? (
          <section className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(0,0.8fr)]">
            <div
              id="mcp-install-zone"
              className="relative overflow-hidden rounded-[28px] border border-[hsl(var(--semantic-extension))]/20 p-5"
              style={{
                background:
                  'radial-gradient(circle at top left, hsl(var(--semantic-extension) / 0.16), transparent 42%), linear-gradient(145deg, hsl(var(--background)) 0%, hsl(var(--semantic-extension) / 0.08) 100%)',
              }}
            >
              <div className="space-y-4">
                <div className="space-y-2">
                  <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                    {t('teams.resource.mcpWorkspace.installEyebrow')}
                  </div>
                  <h3 className="ui-heading text-[26px] leading-none">
                    {t('teams.resource.mcpWorkspace.installTitle')}
                  </h3>
                  <p className="max-w-2xl text-sm leading-6 ui-secondary-text">
                    {t('teams.resource.mcpWorkspace.installDescription')}
                  </p>
                </div>

                <div className="space-y-3">
                  <div className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
                    <div>
                      <div className="text-sm font-semibold text-[hsl(var(--foreground))]">
                        {t('teams.resource.mcpWorkspace.registryTitle')}
                      </div>
                      <p className="mt-1 text-xs leading-5 ui-secondary-text">
                        {t('teams.resource.mcpWorkspace.registryDescription')}
                      </p>
                    </div>
                    <div className="flex items-center gap-2">
                      <div className="relative w-full min-w-[220px] sm:w-[260px]">
                        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-[hsl(var(--muted-foreground))]" />
                        <Input
                          value={templateSearch}
                          onChange={(event) => setTemplateSearch(event.target.value)}
                          placeholder={t('teams.resource.mcpWorkspace.registrySearchPlaceholder')}
                          className="pl-9"
                        />
                      </div>
                      {activeTemplate ? (
                        <Button variant="ghost" size="sm" onClick={resetTemplate}>
                          {t('teams.resource.mcpWorkspace.clearTemplate')}
                        </Button>
                      ) : null}
                    </div>
                  </div>

                  <div className="flex flex-wrap gap-2">
                    {templateFilters.map((filter) => {
                      const active = filter.value === templateCategoryFilter;
                      return (
                        <button
                          key={filter.value}
                          type="button"
                          onClick={() => setTemplateCategoryFilter(filter.value)}
                          className={[
                            'rounded-full border px-3 py-1.5 text-xs font-medium transition-colors',
                            active
                              ? 'border-[hsl(var(--semantic-extension))]/30 bg-[hsl(var(--semantic-extension))]/12 text-[hsl(var(--semantic-extension))]'
                              : 'border-[hsl(var(--border))] bg-[hsl(var(--background))] text-[hsl(var(--muted-foreground))] hover:border-[hsl(var(--semantic-extension))]/18 hover:text-[hsl(var(--foreground))]',
                          ].join(' ')}
                        >
                          {filter.label}
                        </button>
                      );
                    })}
                  </div>

                  <div className="text-xs ui-secondary-text">
                    {t('teams.resource.mcpWorkspace.registryResultCount', {
                      count: filteredTemplates.length,
                    })}
                  </div>

                  <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                    {filteredTemplates.map((template) => {
                      const selected = template.id === activeTemplateId;
                      return (
                        <button
                          key={template.id}
                          type="button"
                          onClick={() => applyTemplate(template)}
                          className={[
                            'rounded-[20px] border px-4 py-4 text-left transition-colors',
                            selected
                              ? 'border-[hsl(var(--semantic-extension))]/35 bg-[hsl(var(--semantic-extension))]/10'
                              : 'border-[hsl(var(--border))] bg-[hsl(var(--background))]/72 hover:border-[hsl(var(--semantic-extension))]/24 hover:bg-[hsl(var(--semantic-extension))]/5',
                          ].join(' ')}
                        >
                          <div className="flex items-start justify-between gap-3">
                            <div className="space-y-2">
                              <div className="inline-flex items-center gap-2 text-[11px] uppercase tracking-[0.16em] text-[hsl(var(--semantic-extension))]">
                                {templateCategoryIcon(template.category)}
                                {templateCategoryLabel(template.category)}
                              </div>
                              <div className="text-sm font-semibold text-[hsl(var(--foreground))]">
                                {template.title}
                              </div>
                            </div>
                            <Badge variant="outline">{formatTransport(template.type)}</Badge>
                          </div>
                          <p className="mt-2 text-xs leading-5 ui-secondary-text">
                            {template.description}
                          </p>
                          <div className="mt-3 text-[11px] leading-5 text-[hsl(var(--semantic-extension))]">
                            {template.note}
                          </div>
                        </button>
                      );
                    })}
                  </div>

                  {filteredTemplates.length === 0 ? (
                    <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/72 px-4 py-4 text-sm ui-secondary-text">
                      {t('teams.resource.mcpWorkspace.registryEmpty')}
                    </div>
                  ) : null}

                  {activeTemplate ? (
                    <div className="rounded-[18px] border border-[hsl(var(--semantic-extension))]/20 bg-[hsl(var(--semantic-extension))]/8 px-4 py-3">
                      <div className="text-xs font-semibold uppercase tracking-[0.16em] text-[hsl(var(--semantic-extension))]">
                        {t('teams.resource.mcpWorkspace.activeTemplate')}
                      </div>
                      <div className="mt-2 text-sm text-[hsl(var(--foreground))]">
                        {activeTemplate.title}
                      </div>
                      <p className="mt-1 text-xs leading-5 ui-secondary-text">
                        {activeTemplate.note}
                      </p>
                    </div>
                  ) : null}
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.installDestination')}
                    </label>
                    <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-4 py-3 text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.installDestinationValue')}
                    </div>
                  </div>
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.transport')}
                    </label>
                    <Select
                      value={form.type}
                      onValueChange={(value) => setForm((prev) => ({ ...prev, type: value as TransportType }))}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="stdio">STDIO</SelectItem>
                        <SelectItem value="sse">SSE</SelectItem>
                        <SelectItem value="streamable_http">Streamable HTTP</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.instanceName')}
                    </label>
                    <Input
                      value={form.name}
                      onChange={(event) => setForm((prev) => ({ ...prev, name: event.target.value }))}
                      placeholder={t('teams.resource.mcpWorkspace.instanceNamePlaceholder')}
                    />
                  </div>
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.attachAfterInstall')}
                    </label>
                    <Select
                      value={form.attachAgentId}
                      onValueChange={(value) => setForm((prev) => ({ ...prev, attachAgentId: value }))}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder={t('teams.resource.mcpWorkspace.attachAfterInstallPlaceholder')} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={NO_ATTACH_AGENT}>
                          {t('teams.resource.mcpWorkspace.attachLater')}
                        </SelectItem>
                        {agents.map((agent) => (
                          <SelectItem key={agent.id} value={agent.id}>
                            {agent.name}
                            {agent.agent_role ? ` · ${formatRole(agent.agent_role)}` : ''}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {form.type === 'stdio'
                        ? t('teams.resource.mcpWorkspace.commandOrPath')
                        : t('teams.resource.mcpWorkspace.endpointUrl')}
                    </label>
                    <Input
                      value={form.uriOrCmd}
                      onChange={(event) => setForm((prev) => ({ ...prev, uriOrCmd: event.target.value }))}
                      placeholder={
                        form.type === 'stdio'
                          ? t('teams.resource.mcpWorkspace.commandPlaceholder')
                          : t('teams.resource.mcpWorkspace.endpointPlaceholder')
                      }
                    />
                  </div>
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.args')}
                    </label>
                    <Textarea
                      value={form.argsText}
                      onChange={(event) => setForm((prev) => ({ ...prev, argsText: event.target.value }))}
                      placeholder={t('teams.resource.mcpWorkspace.argsPlaceholder')}
                      rows={4}
                    />
                  </div>
                  <div className="space-y-2">
                    <label className="text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.envs')}
                    </label>
                    <Textarea
                      value={form.envText}
                      onChange={(event) => setForm((prev) => ({ ...prev, envText: event.target.value }))}
                      placeholder={t('teams.resource.mcpWorkspace.envsPlaceholder')}
                      rows={4}
                    />
                  </div>
                </div>

                <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
                  <div className="text-sm ui-secondary-text">
                    {t('teams.resource.mcpWorkspace.attachHint')}
                  </div>
                  <div className="flex items-center gap-2">
                    <Button variant="outline" onClick={() => void loadWorkspaceData()} disabled={loading}>
                      <RefreshCw className={`mr-2 h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
                      {t('common.reload')}
                    </Button>
                    {canManage ? (
                      <Button
                        className="border-[hsl(var(--semantic-extension))]/24 bg-[hsl(var(--semantic-extension))]/12 text-[hsl(var(--semantic-extension))] shadow-none hover:bg-[hsl(var(--semantic-extension))]/18"
                        onClick={() => void handleInstall()}
                        disabled={submitting}
                      >
                        <Plus className="mr-2 h-4 w-4" />
                        {submitting
                          ? t('teams.resource.mcpWorkspace.installing')
                          : t('teams.resource.mcpWorkspace.installAction')}
                      </Button>
                    ) : null}
                  </div>
                </div>
              </div>
            </div>

            <div className="grid gap-3">
              <div className="rounded-[22px] border border-[hsl(var(--semantic-extension))]/18 bg-[hsl(var(--background))] p-4">
                <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                  {t('teams.resource.mcpWorkspace.planTitle')}
                </div>
                <p className="mt-2 text-sm leading-6 ui-secondary-text">
                  {t('teams.resource.mcpWorkspace.planDescription')}
                </p>
                <div className="mt-4 grid gap-3">
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {t('teams.resource.mcpWorkspace.planTemplate')}
                    </div>
                    <div className="mt-2 text-sm font-medium text-[hsl(var(--foreground))]">
                      {activeTemplate?.title ?? t('teams.resource.mcpWorkspace.planManual')}
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {t('teams.resource.mcpWorkspace.instanceName')}
                    </div>
                    <div className="mt-2 break-all text-sm font-medium text-[hsl(var(--foreground))]">
                      {form.name.trim() || t('teams.resource.mcpWorkspace.planUnset')}
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {t('teams.resource.mcpWorkspace.installDestination')}
                    </div>
                    <div className="mt-2 text-sm font-medium text-[hsl(var(--foreground))]">
                      {t('teams.resource.mcpWorkspace.installDestinationValue')}
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {t('teams.resource.mcpWorkspace.planTargetAgent')}
                    </div>
                    <div className="mt-2 text-sm font-medium text-[hsl(var(--foreground))]">
                      {attachAgent?.name ?? t('teams.resource.mcpWorkspace.planAttachLater')}
                    </div>
                    {attachAgent?.agent_role ? (
                      <div className="mt-1 text-xs ui-secondary-text">
                        {formatRole(attachAgent.agent_role)}
                      </div>
                    ) : null}
                  </div>
                  <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-1">
                    <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                      <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                        {t('teams.resource.mcpWorkspace.planTransport')}
                      </div>
                      <div className="mt-2 text-sm font-medium text-[hsl(var(--foreground))]">
                        {formatTransport(form.type)}
                      </div>
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {form.type === 'stdio'
                        ? t('teams.resource.mcpWorkspace.commandOrPath')
                        : t('teams.resource.mcpWorkspace.endpointUrl')}
                    </div>
                    <div className="mt-2 break-all text-sm font-medium text-[hsl(var(--foreground))]">
                      {form.uriOrCmd || t('teams.resource.mcpWorkspace.planUnset')}
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {t('teams.resource.mcpWorkspace.configSummary')}
                    </div>
                    <div className="mt-2 flex flex-wrap gap-2 text-sm text-[hsl(var(--foreground))]">
                      <span>{t('teams.resource.mcpWorkspace.argsCount', { count: currentArgs.length })}</span>
                      <span className="ui-secondary-text">·</span>
                      <span>{t('teams.resource.mcpWorkspace.envCount', { count: Object.keys(currentEnvs).length })}</span>
                    </div>
                  </div>
                  <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))]/76 px-3 py-3">
                    <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                      {t('teams.resource.mcpWorkspace.planAfterTitle')}
                    </div>
                    <div className="mt-2 space-y-2 text-sm ui-secondary-text">
                      <div>{t('teams.resource.mcpWorkspace.planAfterPersist')}</div>
                      <div>{t('teams.resource.mcpWorkspace.planAfterManage')}</div>
                      <div>{t('teams.resource.mcpWorkspace.planAfterReuse')}</div>
                    </div>
                  </div>
                </div>
              </div>

              <div className="rounded-[22px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] p-4">
                <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                  {t('teams.resource.mcpWorkspace.totalCount')}
                </div>
                <div className="mt-2 text-[32px] font-semibold tracking-tight text-[hsl(var(--foreground))]">
                  {resourceEntries.length}
                </div>
                <p className="mt-2 text-sm ui-secondary-text">
                  {t('teams.resource.mcpWorkspace.overviewDescription')}
                </p>
              </div>
              <div className="grid gap-3 md:grid-cols-3 xl:grid-cols-1">
                <div className="rounded-[22px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] p-4">
                  <div className="flex items-center gap-2 text-sm font-semibold text-[hsl(var(--foreground))]">
                    <Power className="h-4 w-4 text-[hsl(var(--semantic-extension))]" />
                    {t('teams.resource.mcpWorkspace.attachedCount')}
                  </div>
                  <div className="mt-2 text-[24px] font-semibold text-[hsl(var(--foreground))]">{attachedResourceCount}</div>
                </div>
                <div className="rounded-[22px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] p-4">
                  <div className="flex items-center gap-2 text-sm font-semibold text-[hsl(var(--foreground))]">
                    <Terminal className="h-4 w-4 text-[hsl(var(--semantic-extension))]" />
                    {t('teams.resource.mcpWorkspace.protocolStdio')}
                  </div>
                  <div className="mt-2 text-[24px] font-semibold text-[hsl(var(--foreground))]">{stdioCount}</div>
                </div>
                <div className="rounded-[22px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] p-4">
                  <div className="flex items-center gap-2 text-sm font-semibold text-[hsl(var(--foreground))]">
                    <Link2 className="h-4 w-4 text-[hsl(var(--semantic-extension))]" />
                    {t('teams.resource.mcpWorkspace.remoteCount')}
                  </div>
                  <div className="mt-2 text-[24px] font-semibold text-[hsl(var(--foreground))]">{remoteCount}</div>
                </div>
              </div>
            </div>
          </section>
          ) : null}

          {workspaceView === 'manage' ? (
          <section className="space-y-4">
            <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
              <div>
                <h3 className="ui-heading text-[22px] leading-none">
                  {t('teams.resource.mcpWorkspace.manageTitle')}
                </h3>
                <p className="mt-2 text-sm ui-secondary-text">
                  {t('teams.resource.mcpWorkspace.manageDescription')}
                </p>
                <p className="mt-2 text-xs leading-5 ui-tertiary-text">
                  {t('teams.resource.mcpWorkspace.manageModelHint')}
                </p>
              </div>
              <div className="w-full max-w-[320px]">
                <Input
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  placeholder={t('teams.resource.mcpWorkspace.searchPlaceholder')}
                />
              </div>
            </div>

            {error ? (
              <div className="rounded-[18px] border border-[hsl(var(--destructive))]/30 bg-[hsl(var(--destructive))]/6 px-3 py-2 text-sm text-[hsl(var(--destructive))]">
                {error}
              </div>
            ) : null}

            {loading ? (
              <p className="text-sm ui-secondary-text">{t('common.loading')}</p>
            ) : filteredResources.length === 0 ? (
              <div className="ui-empty-panel px-4 py-8 text-center">
                <div className="text-base font-semibold text-[hsl(var(--foreground))]">
                  {t('teams.resource.mcpWorkspace.emptyTitle')}
                </div>
                <p className="mt-2 text-sm ui-secondary-text">
                  {t('teams.resource.mcpWorkspace.emptyDescription')}
                </p>
              </div>
            ) : (
              <div className="space-y-3">
                {pagedResources.map((entry) => (
                  <div key={entry.extension.id} className="ui-subtle-panel p-4">
                    <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                      <div className="min-w-0 space-y-3">
                        <div className="flex flex-wrap items-center gap-2">
                          <div className="text-sm font-semibold text-[hsl(var(--foreground))]">
                            {entry.extension.name}
                          </div>
                          <Badge variant="outline">
                            {formatTransport(entry.extension.extensionType as TransportType)}
                          </Badge>
                          <Badge variant={entry.attachedAgents.length > 0 ? 'secondary' : 'outline'}>
                            {t('teams.resource.mcpWorkspace.attachedAgentCount', { count: entry.attachedAgents.length })}
                          </Badge>
                          {entry.extension.version ? (
                            <Badge variant="outline">v{entry.extension.version}</Badge>
                          ) : null}
                        </div>

                        {entry.extension.description ? (
                          <p className="text-sm leading-6 ui-secondary-text">
                            {entry.extension.description}
                          </p>
                        ) : null}

                        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                          <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3">
                            <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                              {t('teams.resource.mcpWorkspace.installDestination')}
                            </div>
                            <div className="mt-2 text-sm text-[hsl(var(--foreground))]">
                              {t('teams.resource.mcpWorkspace.installDestinationValue')}
                            </div>
                          </div>
                          <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3">
                            <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                              {(entry.extension.extensionType as TransportType) === 'stdio'
                                ? t('teams.resource.mcpWorkspace.commandOrPath')
                                : t('teams.resource.mcpWorkspace.endpointUrl')}
                            </div>
                            <div className="mt-2 break-all text-sm text-[hsl(var(--foreground))]">
                              {getExtensionEntryPoint(entry.extension) ?? t('teams.resource.mcpWorkspace.planUnset')}
                            </div>
                          </div>
                          <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3 md:col-span-2 xl:col-span-1">
                            <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                              {t('teams.resource.mcpWorkspace.configSummary')}
                            </div>
                            <div className="mt-2 flex flex-wrap gap-2 text-sm text-[hsl(var(--foreground))]">
                              <span>{t('teams.resource.mcpWorkspace.argsCount', { count: getExtensionArgsCount(entry.extension) })}</span>
                              <span className="ui-secondary-text">·</span>
                              <span>{t('teams.resource.mcpWorkspace.envCount', { count: getExtensionEnvCount(entry.extension) })}</span>
                            </div>
                          </div>
                        </div>

                        <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3">
                          <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                            {t('teams.resource.mcpWorkspace.attachedAgentsTitle')}
                          </div>
                          {entry.attachedAgents.length === 0 ? (
                            <div className="mt-2 text-sm ui-secondary-text">
                              {t('teams.resource.mcpWorkspace.attachLater')}
                            </div>
                          ) : (
                            <div className="mt-2 flex flex-wrap gap-2">
                              {entry.attachedAgents.map((agent) => (
                                <Badge key={agent.id} variant="outline" className="gap-1">
                                  <Bot className="h-3.5 w-3.5" />
                                  <span>{agent.name}</span>
                                </Badge>
                              ))}
                            </div>
                          )}
                        </div>
                      </div>

                      {canManage ? (
                        <div className="flex items-center gap-2 self-start">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => setAttachExtension(entry.extension)}
                          >
                            <Link2 className="mr-2 h-4 w-4" />
                            {t('teams.resource.mcpWorkspace.attachAction')}
                          </Button>
                        </div>
                      ) : null}
                    </div>
                  </div>
                ))}
              </div>
            )}

            {totalPages > 1 ? (
              <div className="flex items-center justify-between">
                <span className="text-sm ui-secondary-text">
                  {t('common.total')}: {filteredResources.length}
                </span>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={currentPage <= 1}
                    onClick={() => setPage((prev) => prev - 1)}
                  >
                    <ChevronLeft className="h-4 w-4" />
                  </Button>
                  <span className="text-sm">{currentPage} / {totalPages}</span>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={currentPage >= totalPages}
                    onClick={() => setPage((prev) => prev + 1)}
                  >
                    <ChevronRight className="h-4 w-4" />
                  </Button>
                </div>
              </div>
            ) : null}

            {filteredLegacyEntries.length > 0 ? (
              <div className="space-y-3 border-t border-[hsl(var(--border))] pt-6">
                <div>
                  <h4 className="text-base font-semibold text-[hsl(var(--foreground))]">
                    {t('teams.resource.mcpWorkspace.legacyTitle')}
                  </h4>
                  <p className="mt-2 text-sm ui-secondary-text">
                    {t('teams.resource.mcpWorkspace.legacyDescription')}
                  </p>
                </div>
                <div className="space-y-3">
                  {filteredLegacyEntries.map((entry) => (
                    <div key={`${entry.agentId}:${entry.extension.name}`} className="ui-subtle-panel p-4">
                      <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
                        <div className="min-w-0 space-y-3">
                          <div className="flex flex-wrap items-center gap-2">
                            <div className="text-sm font-semibold text-[hsl(var(--foreground))]">
                              {entry.extension.name}
                            </div>
                            <Badge variant="outline">{formatTransport(entry.extension.type)}</Badge>
                            <Badge variant={entry.extension.enabled ? 'secondary' : 'outline'}>
                              {entry.extension.enabled
                                ? t('teams.resource.mcpWorkspace.enabled')
                                : t('teams.resource.mcpWorkspace.disabled')}
                            </Badge>
                          </div>
                          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                            <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3">
                              <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                                {t('teams.resource.mcpWorkspace.targetAgent')}
                              </div>
                              <div className="mt-2 flex items-center gap-2 text-sm text-[hsl(var(--foreground))]">
                                <Bot className="h-4 w-4 text-[hsl(var(--semantic-agent))]" />
                                <span>{entry.agentName}</span>
                              </div>
                            </div>
                            <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3">
                              <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                                {entry.extension.type === 'stdio'
                                  ? t('teams.resource.mcpWorkspace.commandOrPath')
                                  : t('teams.resource.mcpWorkspace.endpointUrl')}
                              </div>
                              <div className="mt-2 break-all text-sm text-[hsl(var(--foreground))]">
                                {entry.extension.uri_or_cmd}
                              </div>
                            </div>
                            <div className="rounded-[18px] border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-3 md:col-span-2 xl:col-span-1">
                              <div className="text-[11px] uppercase tracking-[0.16em] ui-tertiary-text">
                                {t('teams.resource.mcpWorkspace.configSummary')}
                              </div>
                              <div className="mt-2 flex flex-wrap gap-2 text-sm text-[hsl(var(--foreground))]">
                                <span>{t('teams.resource.mcpWorkspace.argsCount', { count: entry.extension.args?.length ?? 0 })}</span>
                                <span className="ui-secondary-text">·</span>
                                <span>{t('teams.resource.mcpWorkspace.envCount', { count: Object.keys(entry.extension.envs ?? {}).length })}</span>
                              </div>
                            </div>
                          </div>
                        </div>

                        {canManage ? (
                          <div className="flex items-center gap-2 self-start">
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => setPendingAction({
                                kind: 'toggle',
                                agentId: entry.agentId,
                                extensionName: entry.extension.name,
                                enabled: !entry.extension.enabled,
                              })}
                            >
                              {entry.extension.enabled ? (
                                <Power className="mr-2 h-4 w-4" />
                              ) : (
                                <Play className="mr-2 h-4 w-4" />
                              )}
                              {entry.extension.enabled
                                ? t('teams.resource.mcpWorkspace.disableAction')
                                : t('teams.resource.mcpWorkspace.enableAction')}
                            </Button>
                            <Button
                              variant="outline"
                              size="sm"
                              onClick={() => setPendingAction({
                                kind: 'remove',
                                agentId: entry.agentId,
                                extensionName: entry.extension.name,
                              })}
                            >
                              <Trash2 className="mr-2 h-4 w-4" />
                              {t('teams.resource.mcpWorkspace.removeAction')}
                            </Button>
                          </div>
                        ) : null}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
          </section>
          ) : null}
        </CardContent>
      </Card>

      <AddExtensionToAgentDialog
        open={!!attachExtension}
        onOpenChange={(open) => {
          if (!open) {
            setAttachExtension(null);
            void loadWorkspaceData();
          }
        }}
        extensionId={attachExtension?.id ?? ''}
        extensionName={attachExtension?.name ?? ''}
        teamId={teamId}
      />

      <ConfirmDialog
        open={!!pendingAction}
        onOpenChange={(open) => {
          if (!open) setPendingAction(null);
        }}
        title={pendingAction?.kind === 'remove'
          ? t('teams.resource.mcpWorkspace.removeConfirmTitle')
          : t('teams.resource.mcpWorkspace.toggleConfirmTitle')}
        description={pendingAction?.kind === 'remove'
          ? t('teams.resource.mcpWorkspace.removeConfirmDescription', { name: pendingAction.extensionName })
          : t('teams.resource.mcpWorkspace.toggleConfirmDescription', {
            name: pendingAction?.extensionName,
            action: pendingAction?.enabled
              ? t('teams.resource.mcpWorkspace.enableAction')
              : t('teams.resource.mcpWorkspace.disableAction'),
          })}
        onConfirm={handleConfirmAction}
        variant={pendingAction?.kind === 'remove' ? 'destructive' : 'default'}
      />
    </>
  );
}
