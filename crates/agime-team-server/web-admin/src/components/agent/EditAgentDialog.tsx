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
import { Eye, EyeOff, RefreshCw } from 'lucide-react';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '../ui/tabs';
import { ExtensionConfigPanel } from './ExtensionConfigPanel';
import { SkillConfigPanel } from './SkillConfigPanel';
import {
  agentApi,
  UpdateAgentRequest,
  ApiFormat,
  AgentAccessMode,
  AgentExtensionConfig,
  AgentSkillConfig,
  CustomExtensionConfig,
  TeamAgent,
} from '../../api/agent';
import { userGroupApi, UserGroupSummary } from '../../api/userGroups';

interface Props {
  agent: TeamAgent | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onUpdated: () => void;
}

export function EditAgentDialog({ agent, open, onOpenChange, onUpdated }: Props) {
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
  const [enabledExtensions, setEnabledExtensions] = useState<AgentExtensionConfig[]>([]);
  const [customExtensions, setCustomExtensions] = useState<CustomExtensionConfig[]>([]);
  const [assignedSkills, setAssignedSkills] = useState<AgentSkillConfig[]>([]);
  // Access control state
  const [accessMode, setAccessMode] = useState<AgentAccessMode>('all');
  const [allowedGroups, setAllowedGroups] = useState<string[]>([]);
  const [deniedGroups, setDeniedGroups] = useState<string[]>([]);
  const [maxConcurrent, setMaxConcurrent] = useState(5);
  const [availableGroups, setAvailableGroups] = useState<UserGroupSummary[]>([]);
  const [reloading, setReloading] = useState(false);

  // Load agent data when dialog opens
  useEffect(() => {
    if (agent && open) {
      setName(agent.name);
      setDescription(agent.description || '');
      setSystemPrompt(agent.system_prompt || '');
      setApiUrl(agent.api_url || '');
      setModel(agent.model || '');
      setApiKey(''); // Don't show existing API key
      setApiFormat(agent.api_format);
      setTemperature(agent.temperature != null ? String(agent.temperature) : '');
      setMaxTokens(agent.max_tokens != null ? String(agent.max_tokens) : '');
      setContextLimit(agent.context_limit != null ? String(agent.context_limit) : '');
      setEnabledExtensions(agent.enabled_extensions || []);
      setCustomExtensions(agent.custom_extensions || []);
      setAssignedSkills(agent.assigned_skills || []);
      setAccessMode(agent.access_mode || 'all');
      setAllowedGroups(agent.allowed_groups || []);
      setDeniedGroups(agent.denied_groups || []);
      setMaxConcurrent(agent.max_concurrent_tasks || 5);
    }
  }, [agent, open]);

  // Load available user groups
  useEffect(() => {
    if (agent && open) {
      userGroupApi.list(agent.team_id).then((res) => {
        setAvailableGroups(res.items);
      }).catch(console.error);
    }
  }, [agent, open]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!agent || !name.trim()) return;

    setLoading(true);
    try {
      const req: UpdateAgentRequest = {
        name: name.trim(),
        description: description.trim() || undefined,
        system_prompt: systemPrompt.trim() || undefined,
        api_url: apiUrl.trim() || undefined,
        model: model.trim() || undefined,
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
        assigned_skills: assignedSkills,
      };
      // Only include api_key if user entered a new one
      if (apiKey.trim()) {
        req.api_key = apiKey.trim();
      }
      await agentApi.updateAgent(agent.id, req);
      onUpdated();
      onOpenChange(false);
    } catch (error) {
      console.error('Failed to update agent:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleReloadExtensions = async () => {
    if (!agent) return;
    setReloading(true);
    try {
      await agentApi.reloadExtensions(agent.id);
    } catch (error) {
      console.error('Failed to reload extensions:', error);
    } finally {
      setReloading(false);
    }
  };

  const toggleGroup = (groupId: string, list: string[], setList: (v: string[]) => void) => {
    if (list.includes(groupId)) {
      setList(list.filter((id) => id !== groupId));
    } else {
      setList([...list, groupId]);
    }
  };

  if (!agent) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t('agent.edit.title')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <Tabs defaultValue="basic" className="w-full">
            <TabsList className="grid w-full grid-cols-4">
              <TabsTrigger value="basic">{t('agent.tabs.basic')}</TabsTrigger>
              <TabsTrigger value="extensions">{t('agent.tabs.extensions')}</TabsTrigger>
              <TabsTrigger value="skills">{t('agent.tabs.skills')}</TabsTrigger>
              <TabsTrigger value="access">{t('agent.tabs.access')}</TabsTrigger>
            </TabsList>

            <TabsContent value="basic" className="space-y-4 py-4">
              <div className="space-y-2">
                <Label htmlFor="edit-name">{t('agent.create.name')} *</Label>
                <Input
                  id="edit-name"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder={t('agent.create.namePlaceholder')}
                  required
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="edit-description">{t('agent.create.description')}</Label>
                <Input
                  id="edit-description"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={t('agent.create.descriptionPlaceholder')}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="editSystemPrompt">{t('agent.create.systemPrompt')}</Label>
                <textarea
                  id="editSystemPrompt"
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
                <Label htmlFor="edit-apiUrl">{t('agent.create.apiUrl')}</Label>
                <Input
                  id="edit-apiUrl"
                  value={apiUrl}
                  onChange={(e) => setApiUrl(e.target.value)}
                  placeholder={t('agent.create.apiUrlPlaceholder')}
                />
              </div>

              <div className="space-y-2">
                <Label htmlFor="edit-model">{t('agent.model')}</Label>
                <Input
                  id="edit-model"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder={t('agent.create.modelPlaceholder')}
                />
              </div>

              {apiFormat !== 'local' && (
                <div className="space-y-2">
                  <Label htmlFor="edit-apiKey">
                    {t('agent.create.apiKey')}
                    <span className="text-xs text-muted-foreground ml-2">
                      ({t('agent.edit.apiKeyHint')})
                    </span>
                  </Label>
                  <div className="relative">
                    <Input
                      id="edit-apiKey"
                      type={showApiKey ? 'text' : 'password'}
                      value={apiKey}
                      onChange={(e) => setApiKey(e.target.value)}
                      placeholder={t('agent.edit.apiKeyPlaceholder')}
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
                    <Label htmlFor="edit-temperature" className="text-xs">{t('agent.create.temperature', 'Temperature')}</Label>
                    <Input
                      id="edit-temperature"
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
                    <Label htmlFor="edit-maxTokens" className="text-xs">{t('agent.create.maxTokens', 'Max Tokens')}</Label>
                    <Input
                      id="edit-maxTokens"
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
                    <Label htmlFor="edit-contextLimit" className="text-xs">{t('agent.create.contextLimit', 'Context Limit')}</Label>
                    <Input
                      id="edit-contextLimit"
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
                teamId={agent?.team_id}
              />
            </TabsContent>

            <TabsContent value="skills" className="py-4">
              {agent && (
                <SkillConfigPanel
                  agentId={agent.id}
                  teamId={agent.team_id}
                  assignedSkills={assignedSkills}
                  onSkillsChange={setAssignedSkills}
                />
              )}
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
                        const selected = accessMode === 'allowlist'
                          ? allowedGroups.includes(g.id)
                          : deniedGroups.includes(g.id);
                        const setter = accessMode === 'allowlist' ? setAllowedGroups : setDeniedGroups;
                        const list = accessMode === 'allowlist' ? allowedGroups : deniedGroups;
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
                <Label htmlFor="edit-maxConcurrent">{t('agent.access.maxConcurrent')}</Label>
                <Input
                  id="edit-maxConcurrent"
                  type="number"
                  min={1}
                  max={100}
                  value={maxConcurrent}
                  onChange={(e) => setMaxConcurrent(parseInt(e.target.value) || 5)}
                />
              </div>

              {/* Reload Extensions */}
              <div className="pt-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={handleReloadExtensions}
                  disabled={reloading}
                >
                  <RefreshCw className={`h-4 w-4 mr-1 ${reloading ? 'animate-spin' : ''}`} />
                  {reloading ? t('agent.access.reloading') : t('agent.access.reload')}
                </Button>
              </div>
            </TabsContent>
          </Tabs>

          <DialogFooter className="mt-4">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={loading || !name.trim()}>
              {loading ? t('common.saving') : t('common.save')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
