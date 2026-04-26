import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { apiClient } from '../../api/client';
import { agentApi } from '../../api/agent';
import { fetchVisibleChatAgents } from '../chat/visibleChatAgents';
import type { TeamWithStats } from '../../api/types';
import type { TeamAgent } from '../../api/agent';

interface SettingsTabProps {
  team: TeamWithStats;
  onUpdate: () => void;
  onDelete: () => void;
}

export function SettingsTab({ team, onUpdate, onDelete }: SettingsTabProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(team.name);
  const [description, setDescription] = useState(team.description || '');
  const [repoUrl, setRepoUrl] = useState(team.repositoryUrl || '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);

  // Document analysis settings state
  const [daLoading, setDaLoading] = useState(true);
  const [daEnabled, setDaEnabled] = useState(true);
  const [daApiUrl, setDaApiUrl] = useState('');
  const [daApiKey, setDaApiKey] = useState('');
  const [daApiKeySet, setDaApiKeySet] = useState(false);
  const [daModel, setDaModel] = useState('');
  const [daApiFormat, setDaApiFormat] = useState('');
  const [daAgentId, setDaAgentId] = useState('');
  const [daMinSize, setDaMinSize] = useState(10);
  const [daMaxSize, setDaMaxSize] = useState('');
  const [daSkipMime, setDaSkipMime] = useState<string[]>([]);
  const [daNewMime, setDaNewMime] = useState('');
  const [daSaving, setDaSaving] = useState(false);
  const [daMsg, setDaMsg] = useState('');
  const [aiDescribeAgentId, setAiDescribeAgentId] = useState('');
  const [aiDescribeSaving, setAiDescribeSaving] = useState(false);
  const [aiDescribeMsg, setAiDescribeMsg] = useState('');
  const [generalAgents, setGeneralAgents] = useState<TeamAgent[]>([]);
  const [generalDefaultAgentId, setGeneralDefaultAgentId] = useState('');
  const [generalAgentSaving, setGeneralAgentSaving] = useState(false);
  const [generalAgentMsg, setGeneralAgentMsg] = useState('');
  const [chatAssistantCompanyName, setChatAssistantCompanyName] = useState('');
  const [chatAssistantDepartmentName, setChatAssistantDepartmentName] = useState('');
  const [chatAssistantTeamName, setChatAssistantTeamName] = useState('');
  const [chatAssistantTeamSummary, setChatAssistantTeamSummary] = useState('');
  const [chatAssistantBusinessContext, setChatAssistantBusinessContext] = useState('');
  const [chatAssistantToneHint, setChatAssistantToneHint] = useState('');
  const [chatAssistantSaving, setChatAssistantSaving] = useState(false);
  const [chatAssistantMsg, setChatAssistantMsg] = useState('');
  const [shellSecurityMode, setShellSecurityMode] = useState<'off' | 'warn' | 'block'>('block');
  const [shellSecuritySaving, setShellSecuritySaving] = useState(false);
  const [shellSecurityMsg, setShellSecurityMsg] = useState('');
  const [agents, setAgents] = useState<TeamAgent[]>([]);

  const isAdmin = team.currentUserRole === 'admin' || team.currentUserRole === 'owner';

  const addMime = () => {
    const trimmed = daNewMime.trim();
    if (trimmed) {
      setDaSkipMime([...daSkipMime, trimmed]);
      setDaNewMime('');
    }
  };

  useEffect(() => {
    loadSettings();
    loadAgents();
  }, [team.id]);

  const loadSettings = async () => {
    try {
      const s = await apiClient.getTeamSettings(team.id);
      const da = s.documentAnalysis;
      setDaEnabled(da.enabled);
      setDaApiUrl(da.apiUrl || '');
      setDaApiKeySet(da.apiKeySet);
      setDaModel(da.model || '');
      setDaApiFormat(da.apiFormat || '');
      setDaAgentId(da.agentId || '');
      setDaMinSize(da.minFileSize);
      setDaMaxSize(da.maxFileSize != null ? String(da.maxFileSize) : '');
      setDaSkipMime(da.skipMimePrefixes);
      setAiDescribeAgentId(s.aiDescribe?.agentId || '');
      setGeneralDefaultAgentId(s.generalAgent?.defaultAgentId || '');
      setChatAssistantCompanyName(s.chatAssistant?.companyName || '');
      setChatAssistantDepartmentName(s.chatAssistant?.departmentName || '');
      setChatAssistantTeamName(s.chatAssistant?.teamName || '');
      setChatAssistantTeamSummary(s.chatAssistant?.teamSummary || '');
      setChatAssistantBusinessContext(s.chatAssistant?.businessContext || '');
      setChatAssistantToneHint(s.chatAssistant?.toneHint || '');
      setShellSecurityMode(s.shellSecurity?.mode || 'block');
    } catch { /* use defaults */ }
    setDaLoading(false);
  };

  const loadAgents = async () => {
    try {
      const [res, visibleAgents] = await Promise.all([
        agentApi.listAgents(team.id, 1, 100),
        fetchVisibleChatAgents(team.id),
      ]);
      setAgents(res.items);
      setGeneralAgents(visibleAgents);
    } catch { /* ignore */ }
  };

  const handleSave = async () => {
    if (!name.trim()) return;
    setSaving(true);
    setError('');
    try {
      await apiClient.updateTeam(team.id, {
        name: name.trim(),
        description: description.trim() || undefined,
        repositoryUrl: repoUrl.trim() || undefined,
      });
      onUpdate();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await apiClient.deleteTeam(team.id);
      onDelete();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
      setDeleting(false);
    }
  };

  const handleSaveDocAnalysis = async () => {
    setDaSaving(true);
    setDaMsg('');
    try {
      await apiClient.updateTeamSettings(team.id, {
        documentAnalysis: {
          enabled: daEnabled,
          apiUrl: daApiUrl || '',
          ...(daApiKey ? { apiKey: daApiKey } : {}),
          model: daModel || '',
          apiFormat: daApiFormat || '',
          agentId: daAgentId || '',
          minFileSize: daMinSize,
          maxFileSize: daMaxSize ? Number(daMaxSize) : null,
          skipMimePrefixes: daSkipMime,
        },
      });
      setDaApiKey('');
      if (daApiKey) setDaApiKeySet(true);
      setDaMsg(t('teams.settings.docAnalysis.saved'));
    } catch (err) {
      setDaMsg(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setDaSaving(false);
    }
  };

  const handleSaveAiDescribe = async () => {
    setAiDescribeSaving(true);
    setAiDescribeMsg('');
    try {
      await apiClient.updateTeamSettings(team.id, {
        aiDescribe: {
          agentId: aiDescribeAgentId || '',
        },
      });
      setAiDescribeMsg(t('teams.settings.aiDescribe.saved'));
    } catch (err) {
      setAiDescribeMsg(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setAiDescribeSaving(false);
    }
  };

  const handleSaveGeneralAgent = async () => {
    setGeneralAgentSaving(true);
    setGeneralAgentMsg('');
    try {
      await apiClient.updateTeamSettings(team.id, {
        generalAgent: {
          defaultAgentId: generalDefaultAgentId || '',
        },
      });
      setGeneralAgentMsg(t('teams.settings.generalAgent.saved'));
    } catch (err) {
      setGeneralAgentMsg(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setGeneralAgentSaving(false);
    }
  };

  const handleSaveChatAssistant = async () => {
    setChatAssistantSaving(true);
    setChatAssistantMsg('');
    try {
      await apiClient.updateTeamSettings(team.id, {
        chatAssistant: {
          companyName: chatAssistantCompanyName || '',
          departmentName: chatAssistantDepartmentName || '',
          teamName: chatAssistantTeamName || '',
          teamSummary: chatAssistantTeamSummary || '',
          businessContext: chatAssistantBusinessContext || '',
          toneHint: chatAssistantToneHint || '',
        },
      });
      setChatAssistantMsg(t('teams.settings.chatAssistant.saved'));
    } catch (err) {
      setChatAssistantMsg(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setChatAssistantSaving(false);
    }
  };

  const handleSaveShellSecurity = async () => {
    setShellSecuritySaving(true);
    setShellSecurityMsg('');
    try {
      await apiClient.updateTeamSettings(team.id, {
        shellSecurity: {
          mode: shellSecurityMode,
        },
      });
      setShellSecurityMsg(t('teams.settings.shellSecurity.saved'));
    } catch (err) {
      setShellSecurityMsg(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setShellSecuritySaving(false);
    }
  };

  return (
    <div className="space-y-6">
      <Card className="ui-section-panel">
        <CardHeader>
          <CardTitle className="ui-heading text-[22px]">{t('teams.settings.teamInfo')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.teamName')} *</label>
            <Input value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.description')}</label>
            <Textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.repoUrl')}</label>
            <Input value={repoUrl} onChange={(e) => setRepoUrl(e.target.value)} />
          </div>
          {error && <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>}
          <Button onClick={handleSave} disabled={saving || !name.trim()}>
            {saving ? t('common.saving') : t('common.save')}
          </Button>
        </CardContent>
      </Card>

      {isAdmin && !daLoading && (
        <Card className="ui-section-panel">
          <CardHeader>
            <CardTitle className="ui-heading text-[22px]">
              {t('teams.settings.generalAgent.title')}
            </CardTitle>
            <p className="ui-secondary-text text-sm">
              {t('teams.settings.generalAgent.description')}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1">
              <label className="text-sm font-medium">
                {t('teams.settings.generalAgent.agent')}
              </label>
              <Select
                value={generalDefaultAgentId || '__unset__'}
                onValueChange={(value) =>
                  setGeneralDefaultAgentId(value === '__unset__' ? '' : value)
                }
              >
                <SelectTrigger>
                  <SelectValue placeholder={t('teams.settings.generalAgent.unset')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__unset__">
                    {t('teams.settings.generalAgent.unset')}
                  </SelectItem>
                  {generalAgents.map((agent) => (
                    <SelectItem key={agent.id} value={agent.id}>
                      {agent.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="ui-tertiary-text text-xs">
                {t('teams.settings.generalAgent.agentHint')}
              </p>
            </div>
            {generalAgentMsg && <p className="text-sm ui-secondary-text">{generalAgentMsg}</p>}
            <Button onClick={handleSaveGeneralAgent} disabled={generalAgentSaving}>
              {generalAgentSaving ? t('common.saving') : t('common.save')}
            </Button>
          </CardContent>
        </Card>
      )}

      {isAdmin && !daLoading && (
        <Card className="ui-section-panel">
          <CardHeader>
            <CardTitle className="ui-heading text-[22px]">
              {t('teams.settings.chatAssistant.title')}
            </CardTitle>
            <p className="ui-secondary-text text-sm">
              {t('teams.settings.chatAssistant.description')}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-4 md:grid-cols-3">
              <div className="space-y-1">
                <label className="text-sm font-medium">
                  {t('teams.settings.chatAssistant.companyName')}
                </label>
                <Input
                  value={chatAssistantCompanyName}
                  onChange={(e) => setChatAssistantCompanyName(e.target.value)}
                  placeholder={t('teams.settings.chatAssistant.companyNamePlaceholder')}
                />
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">
                  {t('teams.settings.chatAssistant.departmentName')}
                </label>
                <Input
                  value={chatAssistantDepartmentName}
                  onChange={(e) => setChatAssistantDepartmentName(e.target.value)}
                  placeholder={t('teams.settings.chatAssistant.departmentNamePlaceholder')}
                />
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">
                  {t('teams.settings.chatAssistant.teamName')}
                </label>
                <Input
                  value={chatAssistantTeamName}
                  onChange={(e) => setChatAssistantTeamName(e.target.value)}
                  placeholder={t('teams.settings.chatAssistant.teamNamePlaceholder')}
                />
              </div>
            </div>
            <div className="space-y-1">
              <label className="text-sm font-medium">
                {t('teams.settings.chatAssistant.teamSummary')}
              </label>
              <Textarea
                value={chatAssistantTeamSummary}
                onChange={(e) => setChatAssistantTeamSummary(e.target.value)}
                className="min-h-[90px]"
                placeholder={t('teams.settings.chatAssistant.teamSummaryPlaceholder')}
              />
            </div>
            <div className="space-y-1">
              <label className="text-sm font-medium">
                {t('teams.settings.chatAssistant.businessContext')}
              </label>
              <Textarea
                value={chatAssistantBusinessContext}
                onChange={(e) => setChatAssistantBusinessContext(e.target.value)}
                className="min-h-[100px]"
                placeholder={t('teams.settings.chatAssistant.businessContextPlaceholder')}
              />
            </div>
            <div className="space-y-1">
              <label className="text-sm font-medium">
                {t('teams.settings.chatAssistant.toneHint')}
              </label>
              <Input
                value={chatAssistantToneHint}
                onChange={(e) => setChatAssistantToneHint(e.target.value)}
                placeholder={t('teams.settings.chatAssistant.toneHintPlaceholder')}
              />
            </div>
            {chatAssistantMsg ? <p className="text-sm text-muted-foreground">{chatAssistantMsg}</p> : null}
            <Button onClick={handleSaveChatAssistant} disabled={chatAssistantSaving}>
              {chatAssistantSaving ? t('common.saving') : t('teams.settings.chatAssistant.save')}
            </Button>
          </CardContent>
        </Card>
      )}

      {isAdmin && !daLoading && (
        <Card className="ui-section-panel">
          <CardHeader>
            <CardTitle className="ui-heading text-[22px]">{t('teams.settings.aiDescribe.title')}</CardTitle>
            <p className="ui-secondary-text text-sm">
              {t('teams.settings.aiDescribe.description')}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1">
              <label className="text-sm font-medium">{t('teams.settings.aiDescribe.agent')}</label>
              <Select
                value={aiDescribeAgentId || '__auto__'}
                onValueChange={(value) => setAiDescribeAgentId(value === '__auto__' ? '' : value)}
              >
                <SelectTrigger>
                  <SelectValue placeholder={t('teams.settings.aiDescribe.agentAuto')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__auto__">{t('teams.settings.aiDescribe.agentAuto')}</SelectItem>
                  {agents.map((a) => (
                    <SelectItem key={a.id} value={a.id}>{a.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="ui-tertiary-text text-xs">{t('teams.settings.aiDescribe.agentHint')}</p>
            </div>
            {aiDescribeMsg && <p className="text-sm ui-secondary-text">{aiDescribeMsg}</p>}
            <Button onClick={handleSaveAiDescribe} disabled={aiDescribeSaving}>
              {aiDescribeSaving ? t('common.saving') : t('common.save')}
            </Button>
          </CardContent>
        </Card>
      )}

      {isAdmin && !daLoading && (
        <Card className="ui-section-panel">
          <CardHeader>
            <CardTitle className="ui-heading text-[22px]">{t('teams.settings.docAnalysis.title')}</CardTitle>
            <p className="ui-secondary-text text-sm">
              {t('teams.settings.docAnalysis.description')}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            <label className="ui-subtle-panel flex items-center gap-3 px-4 py-3">
              <input
                type="checkbox"
                checked={daEnabled}
                onChange={(e) => setDaEnabled(e.target.checked)}
                className="h-4 w-4 rounded border-[hsl(var(--ui-line-strong))/0.78] accent-[hsl(var(--primary))]"
              />
              <span className="text-sm font-medium">{t('teams.settings.docAnalysis.enabled')}</span>
            </label>

            {daEnabled && (
              <>
                {/* Agent selector */}
                <div className="space-y-1">
                  <label className="text-sm font-medium">{t('teams.settings.docAnalysis.agent')}</label>
                  <Select value={daAgentId || '__auto__'} onValueChange={(value) => setDaAgentId(value === '__auto__' ? '' : value)}>
                    <SelectTrigger>
                      <SelectValue placeholder={t('teams.settings.docAnalysis.agentAuto')} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="__auto__">{t('teams.settings.docAnalysis.agentAuto')}</SelectItem>
                      {agents.map((a) => (
                        <SelectItem key={a.id} value={a.id}>{a.name}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className="ui-tertiary-text text-xs">{t('teams.settings.docAnalysis.agentHint')}</p>
                </div>

                {/* Standalone API config */}
                <p className="ui-tertiary-text text-xs italic">{t('teams.settings.docAnalysis.standaloneApiHint')}</p>
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.apiUrl')}</label>
                    <Input value={daApiUrl} onChange={(e) => setDaApiUrl(e.target.value)} placeholder="https://..." />
                  </div>
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.model')}</label>
                    <Input value={daModel} onChange={(e) => setDaModel(e.target.value)} />
                  </div>
                </div>
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-1">
                    <label className="text-sm font-medium">
                      {t('teams.settings.docAnalysis.apiKey')}
                      {daApiKeySet && <span className="ml-2 text-xs text-[hsl(var(--status-success-text))]">({t('teams.settings.docAnalysis.apiKeySet')})</span>}
                    </label>
                    <Input
                      type="password"
                      value={daApiKey}
                      onChange={(e) => setDaApiKey(e.target.value)}
                      placeholder={t('teams.settings.docAnalysis.apiKeyPlaceholder')}
                    />
                  </div>
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.apiFormat')}</label>
                    <Select value={daApiFormat || '__none__'} onValueChange={(value) => setDaApiFormat(value === '__none__' ? '' : value)}>
                      <SelectTrigger>
                        <SelectValue placeholder="-" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__none__">-</SelectItem>
                        <SelectItem value="openai">OpenAI</SelectItem>
                        <SelectItem value="anthropic">Anthropic</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                {/* File size limits */}
                <div className="grid gap-3 md:grid-cols-2">
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.minFileSize')}</label>
                    <Input type="number" value={daMinSize} onChange={(e) => setDaMinSize(Number(e.target.value))} />
                  </div>
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.maxFileSize')}</label>
                    <Input
                      value={daMaxSize}
                      onChange={(e) => setDaMaxSize(e.target.value)}
                      placeholder={t('teams.settings.docAnalysis.maxFileSizePlaceholder')}
                    />
                  </div>
                </div>

                {/* Skip MIME prefixes */}
                <div className="space-y-1">
                  <label className="text-sm font-medium">{t('teams.settings.docAnalysis.skipMime')}</label>
                  <div className="flex flex-wrap gap-2 mb-2">
                    {daSkipMime.map((m) => (
                      <span key={m} className="inline-flex items-center gap-1 rounded-full border border-[hsl(var(--ui-line-soft))/0.66] bg-[hsl(var(--ui-surface-panel-muted))/0.8] px-2.5 py-1 text-xs font-medium text-[hsl(var(--foreground))]">
                        {m}
                        <button onClick={() => setDaSkipMime(daSkipMime.filter((x) => x !== m))} className="ui-inline-action text-[11px] hover:text-[hsl(var(--destructive))]">&times;</button>
                      </span>
                    ))}
                  </div>
                  <div className="flex flex-col gap-2 sm:flex-row">
                    <Input
                      value={daNewMime}
                      onChange={(e) => setDaNewMime(e.target.value)}
                      placeholder="e.g. image/"
                      className="flex-1"
                      onKeyDown={(e) => { if (e.key === 'Enter') addMime(); }}
                    />
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={addMime}
                    >
                      {t('teams.settings.docAnalysis.addMime')}
                    </Button>
                  </div>
                  <p className="ui-tertiary-text text-xs">{t('teams.settings.docAnalysis.skipMimeHint')}</p>
                </div>
              </>
            )}

            {daMsg && <p className="text-sm text-[hsl(var(--status-success-text))]">{daMsg}</p>}
            <Button onClick={handleSaveDocAnalysis} disabled={daSaving}>
              {daSaving ? t('common.saving') : t('common.save')}
            </Button>
          </CardContent>
        </Card>
      )}

      {isAdmin && !daLoading && (
        <Card className="ui-section-panel">
          <CardHeader>
            <CardTitle className="ui-heading text-[22px]">{t('teams.settings.shellSecurity.title')}</CardTitle>
            <p className="ui-secondary-text text-sm">
              {t('teams.settings.shellSecurity.description')}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="space-y-1">
              <label className="text-sm font-medium">{t('teams.settings.shellSecurity.mode')}</label>
              <Select value={shellSecurityMode} onValueChange={(value) => setShellSecurityMode(value as 'off' | 'warn' | 'block')}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="block">{t('teams.settings.shellSecurity.modes.block')}</SelectItem>
                  <SelectItem value="warn">{t('teams.settings.shellSecurity.modes.warn')}</SelectItem>
                  <SelectItem value="off">{t('teams.settings.shellSecurity.modes.off')}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <p className="ui-tertiary-text text-xs">
              {shellSecurityMode === 'block'
                ? t('teams.settings.shellSecurity.modeHints.block')
                : shellSecurityMode === 'warn'
                  ? t('teams.settings.shellSecurity.modeHints.warn')
                  : t('teams.settings.shellSecurity.modeHints.off')}
            </p>
            {shellSecurityMsg && (
              <p className="text-sm ui-secondary-text">{shellSecurityMsg}</p>
            )}
            <Button onClick={handleSaveShellSecurity} disabled={shellSecuritySaving}>
              {shellSecuritySaving ? t('common.saving') : t('common.save')}
            </Button>
          </CardContent>
        </Card>
      )}

      <Card className="ui-section-panel border-[hsl(var(--destructive))/0.38] bg-[hsl(var(--destructive))/0.03]">
        <CardHeader>
          <CardTitle className="text-[hsl(var(--destructive))]">
            {t('teams.settings.dangerZone')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="ui-secondary-text mb-4 text-sm">
            {t('teams.settings.deleteWarning')}
          </p>
          <Button variant="destructive" onClick={() => setDeleteDialogOpen(true)}>
            {t('teams.settings.deleteTeam')}
          </Button>
        </CardContent>
      </Card>

      <Dialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('teams.settings.deleteConfirmTitle')}</DialogTitle>
          </DialogHeader>
          <p className="py-4">{t('teams.settings.deleteConfirmMessage', { name: team.name })}</p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteDialogOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button variant="destructive" onClick={handleDelete} disabled={deleting}>
              {deleting ? t('common.deleting') : t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
