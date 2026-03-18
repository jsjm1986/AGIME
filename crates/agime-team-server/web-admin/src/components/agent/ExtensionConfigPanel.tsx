import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Card, CardHeader, CardTitle, CardContent } from '../ui/card';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Label } from '../ui/label';
import { Badge } from '../ui/badge';
import { Sparkles, ChevronDown, ChevronUp } from 'lucide-react';
import { apiClient } from '../../api/client';
import type { SharedExtension } from '../../api/types';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import {
  AgentExtensionConfig,
  CustomExtensionConfig,
  BUILTIN_EXTENSIONS,
  BuiltinExtension,
} from '../../api/agent';

interface Props {
  enabledExtensions: AgentExtensionConfig[];
  customExtensions: CustomExtensionConfig[];
  onEnabledChange: (extensions: AgentExtensionConfig[]) => void;
  onCustomChange: (extensions: CustomExtensionConfig[]) => void;
  teamId?: string;
}

export function ExtensionConfigPanel({
  enabledExtensions,
  customExtensions,
  onEnabledChange,
  onCustomChange,
  teamId,
}: Props) {
  const { t, i18n } = useTranslation();
  const [addCustomOpen, setAddCustomOpen] = useState(false);
  const [describingId, setDescribingId] = useState<string | null>(null);
  const [descriptions, setDescriptions] = useState<Record<string, string>>({});
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [teamExtensions, setTeamExtensions] = useState<SharedExtension[]>([]);
  const [loadingTeamExtensions, setLoadingTeamExtensions] = useState(false);

  useEffect(() => {
    if (!teamId) {
      setTeamExtensions([]);
      return;
    }
    let cancelled = false;
    const loadTeamExtensions = async () => {
      setLoadingTeamExtensions(true);
      try {
        const response = await apiClient.getExtensions(teamId, {
          page: 1,
          limit: 200,
          sort: 'updated_at',
        });
        if (!cancelled) {
          setTeamExtensions(response.extensions ?? []);
        }
      } catch {
        if (!cancelled) {
          setTeamExtensions([]);
        }
      } finally {
        if (!cancelled) {
          setLoadingTeamExtensions(false);
        }
      }
    };
    void loadTeamExtensions();
    return () => {
      cancelled = true;
    };
  }, [teamId]);

  // Toggle built-in extension
  const toggleExtension = (extId: BuiltinExtension) => {
    const existing = enabledExtensions.find((e) => e.extension === extId);
    if (existing) {
      // Toggle enabled state
      onEnabledChange(
        enabledExtensions.map((e) =>
          e.extension === extId ? { ...e, enabled: !e.enabled } : e
        )
      );
    } else {
      // Add new extension config
      onEnabledChange([...enabledExtensions, { extension: extId, enabled: true }]);
    }
  };

  // Check if extension is enabled
  const isEnabled = (extId: BuiltinExtension) => {
    const config = enabledExtensions.find((e) => e.extension === extId);
    return config?.enabled ?? false;
  };

  // Toggle custom extension
  const toggleCustom = (name: string) => {
    onCustomChange(
      customExtensions.map((e) =>
        e.name === name ? { ...e, enabled: !e.enabled } : e
      )
    );
  };

  // Remove custom extension
  const removeCustom = (name: string) => {
    onCustomChange(customExtensions.filter((e) => e.name !== name));
  };

  const isTeamExtensionAttached = (extension: SharedExtension) =>
    customExtensions.some((item) =>
      item.source_extension_id
        ? item.source_extension_id === extension.id
        : item.source === 'team' && item.name === extension.name
    );

  const buildTeamExtensionConfig = (extension: SharedExtension): CustomExtensionConfig | null => {
    const uriOrCmd = extension.config.uri_or_cmd;
    const uriOrCmdCamel = extension.config.uriOrCmd;
    const command = extension.config.command;
    const entrypoint =
      typeof uriOrCmd === 'string'
        ? uriOrCmd
        : typeof uriOrCmdCamel === 'string'
          ? uriOrCmdCamel
          : typeof command === 'string'
            ? command
            : '';
    if (!entrypoint.trim()) return null;
    const args = Array.isArray(extension.config.args)
      ? extension.config.args.map((value) => String(value))
      : [];
    const envs =
      extension.config.envs && typeof extension.config.envs === 'object' && !Array.isArray(extension.config.envs)
        ? Object.fromEntries(
            Object.entries(extension.config.envs as Record<string, unknown>).map(([key, value]) => [
              key,
              typeof value === 'string' ? value : String(value ?? ''),
            ])
          )
        : {};
    return {
      name: extension.name,
      type: extension.extensionType as CustomExtensionConfig['type'],
      uri_or_cmd: entrypoint.trim(),
      args,
      envs,
      enabled: true,
      source: 'team',
      source_extension_id: extension.id,
    };
  };

  const toggleTeamExtension = (extension: SharedExtension) => {
    if (isTeamExtensionAttached(extension)) {
      onCustomChange(
        customExtensions.filter((item) =>
          item.source_extension_id
            ? item.source_extension_id !== extension.id
            : !(item.source === 'team' && item.name === extension.name)
        )
      );
      return;
    }
    const next = buildTeamExtensionConfig(extension);
    if (!next) return;
    onCustomChange([...customExtensions, next]);
  };

  // AI Describe a built-in extension
  const handleDescribe = async (ext: typeof BUILTIN_EXTENSIONS[number]) => {
    if (!teamId || describingId) return;
    setDescribingId(ext.id);
    try {
      const lang = i18n.language.substring(0, 2);
      const resp = await apiClient.describeBuiltinExtension(teamId, {
        id: ext.id,
        name: ext.name,
        description: ext.description,
        is_platform: ext.isPlatform,
        lang,
      });
      setDescriptions((prev) => ({ ...prev, [ext.id]: resp.description }));
      setExpandedId(ext.id);
    } catch {
      // silently fail - user can retry
    } finally {
      setDescribingId(null);
    }
  };

  // Toggle expanded description
  const toggleExpanded = (extId: string) => {
    setExpandedId((prev) => (prev === extId ? null : extId));
  };

  return (
    <div className="space-y-4">
      {/* Built-in Extensions */}
      <Card>
        <CardHeader className="py-3">
          <CardTitle className="text-sm">{t('agent.extensions.builtin')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          {/* Platform Extensions */}
          <div className="space-y-1">
            <Label className="text-xs text-muted-foreground">
              {t('agent.extensions.platform')}
            </Label>
            <div className="flex flex-wrap gap-2">
              {BUILTIN_EXTENSIONS.filter((e) => e.isPlatform).map((ext) => (
                <div key={ext.id} className="inline-flex items-center gap-1">
                  <Badge
                    variant={isEnabled(ext.id) ? 'default' : 'outline'}
                    className="cursor-pointer"
                    onClick={() => toggleExtension(ext.id)}
                    title={ext.description}
                  >
                    {ext.name}
                  </Badge>
                  {teamId && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-5 w-5"
                      onClick={() => descriptions[ext.id] ? toggleExpanded(ext.id) : handleDescribe(ext)}
                      disabled={describingId === ext.id}
                      title={t('aiInsights.describe')}
                    >
                      {describingId === ext.id ? (
                        <Sparkles className="h-3 w-3 animate-pulse text-status-warning-text" />
                      ) : descriptions[ext.id] ? (
                        expandedId === ext.id ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />
                      ) : (
                        <Sparkles className="h-3 w-3" />
                      )}
                    </Button>
                  )}
                </div>
              ))}
            </div>
            {expandedId && descriptions[expandedId] && BUILTIN_EXTENSIONS.find(e => e.isPlatform && e.id === expandedId) && (
              <div className="mt-2 p-2 rounded bg-muted text-xs whitespace-pre-wrap">
                {descriptions[expandedId]}
              </div>
            )}
          </div>

          {/* MCP Server Extensions */}
          <div className="space-y-1 pt-2">
            <Label className="text-xs text-muted-foreground">
              {t('agent.extensions.mcp')}
            </Label>
            <div className="flex flex-wrap gap-2">
              {BUILTIN_EXTENSIONS.filter((e) => !e.isPlatform).map((ext) => (
                <div key={ext.id} className="inline-flex items-center gap-1">
                  <Badge
                    variant={isEnabled(ext.id) ? 'default' : 'outline'}
                    className="cursor-pointer"
                    onClick={() => toggleExtension(ext.id)}
                    title={ext.description}
                  >
                    {ext.name}
                  </Badge>
                  {teamId && (
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-5 w-5"
                      onClick={() => descriptions[ext.id] ? toggleExpanded(ext.id) : handleDescribe(ext)}
                      disabled={describingId === ext.id}
                      title={t('aiInsights.describe')}
                    >
                      {describingId === ext.id ? (
                        <Sparkles className="h-3 w-3 animate-pulse text-status-warning-text" />
                      ) : descriptions[ext.id] ? (
                        expandedId === ext.id ? <ChevronUp className="h-3 w-3" /> : <ChevronDown className="h-3 w-3" />
                      ) : (
                        <Sparkles className="h-3 w-3" />
                      )}
                    </Button>
                  )}
                </div>
              ))}
            </div>
            {expandedId && descriptions[expandedId] && BUILTIN_EXTENSIONS.find(e => !e.isPlatform && e.id === expandedId) && (
              <div className="mt-2 p-2 rounded bg-muted text-xs whitespace-pre-wrap">
                {descriptions[expandedId]}
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      {/* Custom Extensions */}
      <Card>
        <CardHeader className="py-3 flex flex-row items-center justify-between">
          <CardTitle className="text-sm">{t('agent.extensions.custom')}</CardTitle>
          <Button type="button" size="sm" variant="outline" onClick={() => setAddCustomOpen(true)}>
            {t('agent.extensions.addCustom')}
          </Button>
        </CardHeader>
        <CardContent>
          {teamId && (
            <div className="space-y-2 pb-4 border-b border-[hsl(var(--border))]">
              <div className="space-y-1">
                <Label className="text-xs text-muted-foreground">
                  {t('agent.extensions.teamLibrary', '团队扩展库')}
                </Label>
                <p className="text-xs text-muted-foreground">
                  {t(
                    'agent.extensions.teamLibraryHint',
                    '这里显示团队扩展库里的 MCP/扩展资源，点击即可挂载到当前 Agent。'
                  )}
                </p>
              </div>
              {loadingTeamExtensions ? (
                <p className="text-sm text-muted-foreground">{t('common.loading')}</p>
              ) : teamExtensions.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {t('agent.extensions.noTeamLibrary', '团队扩展库里还没有可挂载的资源。')}
                </p>
              ) : (
                <div className="flex flex-wrap gap-2">
                  {teamExtensions.map((extension) => {
                    const attached = isTeamExtensionAttached(extension);
                    return (
                      <Badge
                        key={extension.id}
                        variant={attached ? 'default' : 'outline'}
                        className="cursor-pointer"
                        onClick={() => toggleTeamExtension(extension)}
                        title={extension.description || extension.name}
                      >
                        {extension.name}
                      </Badge>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {customExtensions.length === 0 ? (
            <p className="text-sm text-muted-foreground text-center py-2">
              {t('agent.extensions.noCustom')}
            </p>
          ) : (
            <div className={`space-y-2 ${teamId ? 'pt-4' : ''}`}>
              {customExtensions.map((ext) => (
                <div
                  key={ext.name}
                  className="flex items-center justify-between p-2 border rounded"
                >
                  <div className="flex items-center gap-2">
                    <Badge
                      variant={ext.enabled ? 'default' : 'outline'}
                      className="cursor-pointer"
                      onClick={() => toggleCustom(ext.name)}
                    >
                      {ext.name}
                    </Badge>
                    <span className="text-xs text-muted-foreground">
                      ({ext.type})
                    </span>
                    {ext.source === 'team' && (
                      <Badge variant="secondary" className="text-xs">
                        Team
                      </Badge>
                    )}
                  </div>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => removeCustom(ext.name)}
                  >
                    ×
                  </Button>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Add Custom Extension Dialog */}
      <AddCustomExtensionDialog
        open={addCustomOpen}
        onOpenChange={setAddCustomOpen}
        onAdd={(ext) => {
          onCustomChange([...customExtensions, ext]);
          setAddCustomOpen(false);
        }}
      />
    </div>
  );
}

// Add Custom Extension Dialog
interface AddCustomDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd: (ext: CustomExtensionConfig) => void;
}

function AddCustomExtensionDialog({ open, onOpenChange, onAdd }: AddCustomDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [type, setType] = useState<'sse' | 'stdio'>('stdio');
  const [uriOrCmd, setUriOrCmd] = useState('');
  const [args, setArgs] = useState('');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !uriOrCmd.trim()) return;

    onAdd({
      name: name.trim(),
      type,
      uri_or_cmd: uriOrCmd.trim(),
      args: args.trim() ? args.split(' ') : [],
      envs: {},
      enabled: true,
    });

    // Reset form
    setName('');
    setType('stdio');
    setUriOrCmd('');
    setArgs('');
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[400px]">
        <DialogHeader>
          <DialogTitle>{t('agent.extensions.addCustomTitle')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label>{t('agent.extensions.name')}</Label>
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-extension"
                required
              />
            </div>

            <div className="space-y-2">
              <Label>{t('agent.extensions.type')}</Label>
              <Select value={type} onValueChange={(v) => setType(v as 'sse' | 'stdio')}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="stdio">Stdio (Command)</SelectItem>
                  <SelectItem value="sse">SSE (HTTP)</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>
                {type === 'stdio'
                  ? t('agent.extensions.command')
                  : t('agent.extensions.url')}
              </Label>
              <Input
                value={uriOrCmd}
                onChange={(e) => setUriOrCmd(e.target.value)}
                placeholder={
                  type === 'stdio' ? 'npx @modelcontextprotocol/server-xxx' : 'http://localhost:3000/sse'
                }
                required
              />
            </div>

            {type === 'stdio' && (
              <div className="space-y-2">
                <Label>{t('agent.extensions.args')}</Label>
                <Input
                  value={args}
                  onChange={(e) => setArgs(e.target.value)}
                  placeholder="--arg1 value1 --arg2 value2"
                />
              </div>
            )}
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={!name.trim() || !uriOrCmd.trim()}>
              {t('common.add')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
