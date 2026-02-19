import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Label } from '../ui/label';
import { Eye, EyeOff } from 'lucide-react';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/tabs';
import { ExtensionConfigPanel } from './ExtensionConfigPanel';
import {
  agentApi,
  CreateAgentRequest,
  ApiFormat,
  AgentAccessMode,
  AgentExtensionConfig,
  CustomExtensionConfig,
  DEFAULT_EXTENSIONS,
} from '../../api/agent';
import { userGroupApi, UserGroupSummary } from '../../api/userGroups';

interface Props {
  teamId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}

export function CreateAgentDialog({ teamId, open, onOpenChange, onCreated }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [apiUrl, setApiUrl] = useState('');
  const [model, setModel] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [apiFormat, setApiFormat] = useState<ApiFormat>('openai');
  const [showApiKey, setShowApiKey] = useState(false);
  const [temperature, setTemperature] = useState<string>('');
  const [maxTokens, setMaxTokens] = useState<string>('');
  const [contextLimit, setContextLimit] = useState<string>('');

  // Extension configuration state
  const [enabledExtensions, setEnabledExtensions] = useState<AgentExtensionConfig[]>(
    DEFAULT_EXTENSIONS.map((ext) => ({ extension: ext, enabled: true }))
  );
  const [customExtensions, setCustomExtensions] = useState<CustomExtensionConfig[]>([]);
  // Access control state
  const [accessMode, setAccessMode] = useState<AgentAccessMode>('all');
  const [allowedGroups, setAllowedGroups] = useState<string[]>([]);
  const [deniedGroups, setDeniedGroups] = useState<string[]>([]);
  const [maxConcurrent, setMaxConcurrent] = useState(5);
  const [availableGroups, setAvailableGroups] = useState<UserGroupSummary[]>([]);

  // Load available user groups
  useEffect(() => {
    if (open) {
      userGroupApi.list(teamId).then((res) => {
        setAvailableGroups(res.items);
      }).catch(console.error);
    }
  }, [teamId, open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    setLoading(true);
    try {
      const req: CreateAgentRequest = {
        team_id: teamId,
        name: name.trim(),
        description: description.trim() || undefined,
        system_prompt: systemPrompt.trim() || undefined,
        api_url: apiUrl.trim() || undefined,
        model: model.trim() || undefined,
        api_key: apiKey.trim() || undefined,
        api_format: apiFormat,
        enabled_extensions: enabledExtensions,
        custom_extensions: customExtensions,
        access_mode: accessMode,
        allowed_groups: allowedGroups,
        denied_groups: deniedGroups,
        max_concurrent_tasks: maxConcurrent,
        temperature: temperature ? parseFloat(temperature) : undefined,
        max_tokens: maxTokens ? parseInt(maxTokens) : undefined,
        context_limit: contextLimit ? parseInt(contextLimit) : undefined,
      };
      await agentApi.createAgent(req);
      onCreated();
      onOpenChange(false);
      resetForm();
    } catch (error) {
      console.error('Failed to create agent:', error);
    } finally {
      setLoading(false);
    }
  };

  const resetForm = () => {
    setName('');
    setDescription('');
    setSystemPrompt('');
    setApiUrl('');
    setModel('');
    setApiKey('');
    setApiFormat('openai');
    setEnabledExtensions(DEFAULT_EXTENSIONS.map((ext) => ({ extension: ext, enabled: true })));
    setCustomExtensions([]);
    setAccessMode('all');
    setAllowedGroups([]);
    setDeniedGroups([]);
    setMaxConcurrent(5);
    setTemperature('');
    setMaxTokens('');
    setContextLimit('');
    setShowApiKey(false);
  };

  const toggleGroup = (groupId: string, list: string[], setList: (v: string[]) => void) => {
    if (list.includes(groupId)) {
      setList(list.filter((id) => id !== groupId));
    } else {
      setList([...list, groupId]);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t('agent.create.title')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <Tabs defaultValue="basic" className="w-full">
            <TabsList className="grid w-full grid-cols-3">
              <TabsTrigger value="basic">{t('agent.tabs.basic')}</TabsTrigger>
              <TabsTrigger value="extensions">{t('agent.tabs.extensions')}</TabsTrigger>
              <TabsTrigger value="access">{t('agent.tabs.access')}</TabsTrigger>
            </TabsList>

            <TabsContent value="basic" className="space-y-4 py-4">
              <div className="space-y-2">
                <Label htmlFor="name">{t('agent.create.name')} *</Label>
                <Input
                  id="name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={t('agent.create.namePlaceholder')}
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="description">{t('agent.create.description')}</Label>
                <Input
                  id="description"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={t('agent.create.descriptionPlaceholder')}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="systemPrompt">{t('agent.create.systemPrompt')}</Label>
                <textarea
                  id="systemPrompt"
                  className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                  value={systemPrompt}
                  onChange={(e) => setSystemPrompt(e.target.value)}
                  placeholder={t('agent.create.systemPromptPlaceholder')}
                  rows={3}
                />
              </div>

              <div className="space-y-2">
                <Label>{t('agent.create.apiFormat')}</Label>
                <Select value={apiFormat} onValueChange={(v) => setApiFormat(v as ApiFormat)}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openai">OpenAI {t('agent.create.compatible')}</SelectItem>
                    <SelectItem value="anthropic">Anthropic {t('agent.create.compatible')}</SelectItem>
                    <SelectItem value="local">{t('agent.create.localModel')}</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="apiUrl">{t('agent.create.apiUrl')}</Label>
                <Input
                  id="apiUrl"
                  value={apiUrl}
                  onChange={(e) => setApiUrl(e.target.value)}
                  placeholder={t('agent.create.apiUrlPlaceholder')}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="model">{t('agent.model')}</Label>
                <Input
                  id="model"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={t('agent.create.modelPlaceholder')}
                />
              </div>

              {apiFormat !== 'local' && (
                <div className="space-y-2">
                  <Label htmlFor="apiKey">{t('agent.create.apiKey')}</Label>
                  <div className="relative">
                    <Input
                      id="apiKey"
                      type={showApiKey ? 'text' : 'password'}
                      value={apiKey}
                      onChange={(e) => setApiKey(e.target.value)}
                      placeholder={t('agent.create.apiKeyPlaceholder')}
                      className="pr-10"
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="absolute right-0 top-0 h-full px-3 hover:bg-transparent"
                      onClick={() => setShowApiKey(!showApiKey)}
                    >
                      {showApiKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </Button>
                  </div>
                </div>
              )}

              {/* Advanced LLM Settings */}
              <div className="pt-2 border-t">
                <p className="text-xs text-muted-foreground mb-3">{t('agent.create.advancedSettings', 'Advanced Settings (optional)')}</p>
                <div className="grid grid-cols-3 gap-3">
                  <div className="space-y-1">
                    <Label htmlFor="temperature" className="text-xs">{t('agent.create.temperature', 'Temperature')}</Label>
                    <Input
                      id="temperature"
                      type="number"
                      step="0.1"
                      min="0"
                      max="2"
                      value={temperature}
                      onChange={(e) => setTemperature(e.target.value)}
                      placeholder="0.7"
                      className="h-8 text-sm"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="maxTokens" className="text-xs">{t('agent.create.maxTokens', 'Max Tokens')}</Label>
                    <Input
                      id="maxTokens"
                      type="number"
                      min="1"
                      max="200000"
                      value={maxTokens}
                      onChange={(e) => setMaxTokens(e.target.value)}
                      placeholder="4096"
                      className="h-8 text-sm"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label htmlFor="contextLimit" className="text-xs">{t('agent.create.contextLimit', 'Context Limit')}</Label>
                    <Input
                      id="contextLimit"
                      type="number"
                      min="1024"
                      max="2000000"
                      value={contextLimit}
                      onChange={(e) => setContextLimit(e.target.value)}
                      placeholder="128000"
                      className="h-8 text-sm"
                    />
                  </div>
                </div>
              </div>
            </TabsContent>

            <TabsContent value="extensions" className="py-4">
              <ExtensionConfigPanel
                enabledExtensions={enabledExtensions}
                customExtensions={customExtensions}
                onEnabledChange={setEnabledExtensions}
                onCustomChange={setCustomExtensions}
                teamId={teamId}
              />
            </TabsContent>

            <TabsContent value="access" className="space-y-4 py-4">
              {/* Access Mode */}
              <div className="space-y-2">
                <Label>{t('agent.access.mode')}</Label>
                <Select value={accessMode} onValueChange={(v) => setAccessMode(v as AgentAccessMode)}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t('agent.access.all')}</SelectItem>
                    <SelectItem value="allowlist">{t('agent.access.allowlist')}</SelectItem>
                    <SelectItem value="denylist">{t('agent.access.denylist')}</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {accessMode === 'all' && t('agent.access.allDesc')}
                  {accessMode === 'allowlist' && t('agent.access.allowlistDesc')}
                  {accessMode === 'denylist' && t('agent.access.denylistDesc')}
                </p>
              </div>

              {/* Group Selection */}
              {accessMode !== 'all' && (
                <div className="space-y-2">
                  <Label>
                    {accessMode === 'allowlist'
                      ? t('agent.access.allowedGroups')
                      : t('agent.access.deniedGroups')}
                  </Label>
                  {availableGroups.length === 0 ? (
                    <p className="text-sm text-muted-foreground">{t('agent.access.noGroups')}</p>
                  ) : (
                    <div className="space-y-1 max-h-40 overflow-y-auto border rounded p-2">
                      {availableGroups.map((g) => {
                        const list = accessMode === 'allowlist' ? allowedGroups : deniedGroups;
                        const setter = accessMode === 'allowlist' ? setAllowedGroups : setDeniedGroups;
                        const selected = list.includes(g.id);
                        return (
                          <label key={g.id} className="flex items-center gap-2 py-1 px-1 rounded hover:bg-muted cursor-pointer">
                            <input
                              type="checkbox"
                              checked={selected}
                              onChange={() => toggleGroup(g.id, list, setter)}
                              className="rounded"
                            />
                            <span className="text-sm">{g.name}</span>
                            <span className="text-xs text-muted-foreground ml-auto">
                              {t('userGroups.memberCount', { count: g.memberCount })}
                            </span>
                          </label>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}

              {/* Max Concurrent Tasks */}
              <div className="space-y-2">
                <Label htmlFor="create-maxConcurrent">{t('agent.access.maxConcurrent')}</Label>
                <Input
                  id="create-maxConcurrent"
                  type="number"
                  min={1}
                  max={100}
                  value={maxConcurrent}
                  onChange={(e) => setMaxConcurrent(parseInt(e.target.value) || 5)}
                />
              </div>
            </TabsContent>
          </Tabs>

          <DialogFooter className="mt-4">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={loading || !name.trim()}>
              {loading ? t('common.creating') : t('common.create')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
