import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { apiClient } from '../../api/client';
import { agentApi } from '../../api/agent';
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
    } catch { /* use defaults */ }
    setDaLoading(false);
  };

  const loadAgents = async () => {
    try {
      const res = await agentApi.listAgents(team.id, 1, 100);
      setAgents(res.items);
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

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>{t('teams.settings.teamInfo')}</CardTitle>
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

      {/* Document Analysis Settings */}
      {isAdmin && !daLoading && (
        <Card>
          <CardHeader>
            <CardTitle>{t('teams.settings.docAnalysis.title')}</CardTitle>
            <p className="text-sm text-[hsl(var(--muted-foreground))]">
              {t('teams.settings.docAnalysis.description')}
            </p>
          </CardHeader>
          <CardContent className="space-y-4">
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={daEnabled}
                onChange={(e) => setDaEnabled(e.target.checked)}
                className="rounded"
              />
              <span className="text-sm font-medium">{t('teams.settings.docAnalysis.enabled')}</span>
            </label>

            {daEnabled && (
              <>
                {/* Agent selector */}
                <div className="space-y-1">
                  <label className="text-sm font-medium">{t('teams.settings.docAnalysis.agent')}</label>
                  <select
                    value={daAgentId}
                    onChange={(e) => setDaAgentId(e.target.value)}
                    className="w-full rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-2 text-sm"
                  >
                    <option value="">{t('teams.settings.docAnalysis.agentAuto')}</option>
                    {agents.map((a) => (
                      <option key={a.id} value={a.id}>{a.name}</option>
                    ))}
                  </select>
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">{t('teams.settings.docAnalysis.agentHint')}</p>
                </div>

                {/* Standalone API config */}
                <p className="text-xs text-[hsl(var(--muted-foreground))] italic">{t('teams.settings.docAnalysis.standaloneApiHint')}</p>
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.apiUrl')}</label>
                    <Input value={daApiUrl} onChange={(e) => setDaApiUrl(e.target.value)} placeholder="https://..." />
                  </div>
                  <div className="space-y-1">
                    <label className="text-sm font-medium">{t('teams.settings.docAnalysis.model')}</label>
                    <Input value={daModel} onChange={(e) => setDaModel(e.target.value)} />
                  </div>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1">
                    <label className="text-sm font-medium">
                      {t('teams.settings.docAnalysis.apiKey')}
                      {daApiKeySet && <span className="ml-2 text-xs text-green-600">({t('teams.settings.docAnalysis.apiKeySet')})</span>}
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
                    <select
                      value={daApiFormat}
                      onChange={(e) => setDaApiFormat(e.target.value)}
                      className="w-full rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] px-3 py-2 text-sm"
                    >
                      <option value="">-</option>
                      <option value="openai">OpenAI</option>
                      <option value="anthropic">Anthropic</option>
                    </select>
                  </div>
                </div>

                {/* File size limits */}
                <div className="grid grid-cols-2 gap-3">
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
                      <span key={m} className="inline-flex items-center gap-1 px-2 py-1 rounded bg-[hsl(var(--muted))] text-xs">
                        {m}
                        <button onClick={() => setDaSkipMime(daSkipMime.filter((x) => x !== m))} className="hover:text-[hsl(var(--destructive))]">&times;</button>
                      </span>
                    ))}
                  </div>
                  <div className="flex gap-2">
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
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">{t('teams.settings.docAnalysis.skipMimeHint')}</p>
                </div>
              </>
            )}

            {daMsg && <p className="text-sm text-green-600">{daMsg}</p>}
            <Button onClick={handleSaveDocAnalysis} disabled={daSaving}>
              {daSaving ? t('common.saving') : t('common.save')}
            </Button>
          </CardContent>
        </Card>
      )}

      <Card className="border-[hsl(var(--destructive))]">
        <CardHeader>
          <CardTitle className="text-[hsl(var(--destructive))]">
            {t('teams.settings.dangerZone')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-[hsl(var(--muted-foreground))] mb-4">
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
