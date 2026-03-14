import { Fragment, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useToast } from '../../contexts/ToastContext';
import {
  Bot,
  ChevronLeft,
  ChevronRight,
  Download,
  Eye,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
  X,
} from 'lucide-react';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../ui/card';
import { Input } from '../ui/input';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Textarea } from '../ui/textarea';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '../ui/table';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { ResourceDetailDialog } from './ResourceDetailDialog';
import { CreateSkillDialog } from './CreateSkillDialog';
import { AddSkillToAgentDialog } from './AddSkillToAgentDialog';
import { apiClient } from '../../api/client';
import type {
  ImportedRegistrySkillSummary,
  SharedSkill,
  SkillRegistryPreviewResponse,
  SkillRegistrySearchItem,
  SkillRegistryUpdateInspection,
} from '../../api/types';
import { formatDateTime } from '../../utils/format';

type ConfirmAction = { type: 'delete' | 'uninstall'; id: string };

interface SkillsTabProps {
  teamId: string;
  canManage: boolean;
}

export function SkillsTab({ teamId, canManage }: SkillsTabProps) {
  const { t, i18n } = useTranslation();
  const { addToast } = useToast();
  const [skills, setSkills] = useState<SharedSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedSkill, setSelectedSkill] = useState<SharedSkill | null>(null);
  const [dialogMode, setDialogMode] = useState<'view' | 'edit'>('view');
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState('updated_at');
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(1);
  const [total, setTotal] = useState(0);
  const [addToAgentSkill, setAddToAgentSkill] = useState<{ id: string; name: string } | null>(null);
  const [describingId, setDescribingId] = useState<string | null>(null);
  const [expandedDescriptions, setExpandedDescriptions] = useState<Set<string>>(new Set());
  const [fetchingSkillId, setFetchingSkillId] = useState<string | null>(null);
  const [backfilling, setBackfilling] = useState(false);
  const [confirmAction, setConfirmAction] = useState<ConfirmAction | null>(null);

  const [registryQuery, setRegistryQuery] = useState('');
  const [registrySearching, setRegistrySearching] = useState(false);
  const [registrySearchPerformed, setRegistrySearchPerformed] = useState(false);
  const [registryResults, setRegistryResults] = useState<SkillRegistrySearchItem[]>([]);
  const [registryImportedSkills, setRegistryImportedSkills] = useState<ImportedRegistrySkillSummary[]>([]);
  const [registryImportedLoading, setRegistryImportedLoading] = useState(true);
  const [registryInlineError, setRegistryInlineError] = useState('');
  const [previewLoadingKey, setPreviewLoadingKey] = useState<string | null>(null);
  const [previewSkill, setPreviewSkill] = useState<SkillRegistryPreviewResponse | null>(null);
  const [importingRegistryKey, setImportingRegistryKey] = useState<string | null>(null);
  const [checkingRegistryUpdates, setCheckingRegistryUpdates] = useState(false);
  const [registryUpdatesLoaded, setRegistryUpdatesLoaded] = useState(false);
  const [registryUpdates, setRegistryUpdates] = useState<Record<string, SkillRegistryUpdateInspection>>({});
  const [upgradingRegistrySkillId, setUpgradingRegistrySkillId] = useState<string | null>(null);

  function getErrorMsg(err: unknown, fallbackKey = 'common.error'): string {
    return err instanceof Error ? err.message : t(fallbackKey);
  }

  const loadSkills = useCallback(async () => {
    try {
      setLoading(true);
      const response = await apiClient.getSkills(teamId, {
        page,
        limit: 20,
        search: search || undefined,
        sort,
      });
      setSkills(response.skills);
      setTotalPages(response.total_pages ?? 1);
      setTotal(response.total);
      setError('');
    } catch (err) {
      setError(getErrorMsg(err));
    } finally {
      setLoading(false);
    }
  }, [teamId, page, search, sort, t]);

  const loadImportedRegistrySkills = useCallback(async () => {
    try {
      setRegistryImportedLoading(true);
      const response = await apiClient.getImportedRegistrySkills(teamId);
      setRegistryImportedSkills(response.skills);
      setRegistryInlineError('');
      setRegistryUpdatesLoaded(false);
      setRegistryUpdates({});
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setRegistryImportedLoading(false);
    }
  }, [teamId, t]);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  useEffect(() => {
    loadImportedRegistrySkills();
  }, [loadImportedRegistrySkills]);

  useEffect(() => {
    setSearch('');
    setSort('updated_at');
    setPage(1);
    setSelectedSkill(null);
    setAddToAgentSkill(null);
    setExpandedDescriptions(new Set());
    setError('');
    setRegistryQuery('');
    setRegistryResults([]);
    setRegistrySearchPerformed(false);
    setPreviewSkill(null);
    setRegistryInlineError('');
    setRegistryUpdates({});
    setRegistryUpdatesLoaded(false);
  }, [teamId]);

  useEffect(() => {
    setPage(1);
  }, [search, sort]);

  const handleConfirm = async () => {
    if (!confirmAction) return;
    try {
      if (confirmAction.type === 'delete') {
        await apiClient.deleteSkill(confirmAction.id);
      } else {
        await apiClient.uninstallSkill(confirmAction.id);
      }
      loadSkills();
    } catch (err) {
      setError(getErrorMsg(err));
    } finally {
      setConfirmAction(null);
    }
  };

  const handleInstall = async (skillId: string) => {
    setInstallingId(skillId);
    try {
      const result = await apiClient.installSkill(skillId);
      if (result.success) {
        addToast('success', t('teams.resource.installSuccess'));
      } else {
        setError(result.error || t('teams.resource.installFailed'));
      }
    } catch (err) {
      setError(getErrorMsg(err));
    } finally {
      setInstallingId(null);
    }
  };

  const handleAiDescribe = async (skillId: string) => {
    if (describingId) return;
    const skill = skills.find((item) => item.id === skillId);
    if (skill?.aiDescription && skill.aiDescriptionLang === i18n.language.substring(0, 2)) {
      setExpandedDescriptions((prev) => {
        const next = new Set(prev);
        if (next.has(skillId)) next.delete(skillId); else next.add(skillId);
        return next;
      });
      return;
    }
    setDescribingId(skillId);
    try {
      const lang = i18n.language.substring(0, 2);
      const result = await apiClient.describeSkill(teamId, skillId, lang);
      setSkills((prev) => prev.map((item) => (
        item.id === skillId
          ? { ...item, aiDescription: result.description, aiDescriptionLang: result.lang, aiDescribedAt: result.generated_at }
          : item
      )));
      setExpandedDescriptions((prev) => new Set(prev).add(skillId));
    } catch (err) {
      setError(getErrorMsg(err, 'aiInsights.generateError'));
    } finally {
      setDescribingId(null);
    }
  };

  const handleViewOrEdit = async (skill: SharedSkill, mode: 'view' | 'edit') => {
    setFetchingSkillId(skill.id);
    try {
      const fullSkill = await apiClient.getSkill(skill.id);
      setSelectedSkill(fullSkill);
      setDialogMode(mode);
    } catch (err) {
      setError(getErrorMsg(err));
    } finally {
      setFetchingSkillId(null);
    }
  };

  const handleBackfill = async () => {
    setBackfilling(true);
    try {
      const result = await apiClient.backfillSkillMd(teamId);
      addToast('success', t('teams.resource.backfillSuccess', { count: result.updated }));
      loadSkills();
    } catch (err) {
      setError(getErrorMsg(err));
    } finally {
      setBackfilling(false);
    }
  };

  const handleRegistrySearch = async () => {
    const trimmed = registryQuery.trim();
    if (!trimmed) {
      setRegistryResults([]);
      setRegistrySearchPerformed(false);
      return;
    }
    setRegistrySearching(true);
    setRegistryInlineError('');
    try {
      const response = await apiClient.searchSkillRegistry(teamId, trimmed, 12);
      setRegistryResults(response.skills);
      setRegistrySearchPerformed(true);
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setRegistrySearching(false);
    }
  };

  const handleRegistryPreview = async (result: SkillRegistrySearchItem) => {
    const key = `${result.source}:${result.skill_id}`;
    setPreviewLoadingKey(key);
    setRegistryInlineError('');
    try {
      const response = await apiClient.previewSkillRegistrySkill({
        teamId,
        source: result.source,
        skillId: result.skill_id,
      });
      setPreviewSkill(response);
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setPreviewLoadingKey(null);
    }
  };

  const handleRegistryImport = async (source: string, skillId: string, sourceRef?: string) => {
    const key = `${source}:${skillId}`;
    setImportingRegistryKey(key);
    setRegistryInlineError('');
    try {
      const response = await apiClient.importSkillRegistrySkill({
        teamId,
        source,
        skillId,
        sourceRef,
      });
      addToast('success', t('teams.resource.skillRegistry.importSuccess', { name: response.name }));
      await Promise.all([loadSkills(), loadImportedRegistrySkills()]);
      setPreviewSkill((prev) => (
        prev && prev.source === source && prev.skill_id === skillId
          ? { ...prev, already_imported: true }
          : prev
      ));
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setImportingRegistryKey(null);
    }
  };

  const handleCheckRegistryUpdates = async () => {
    setCheckingRegistryUpdates(true);
    setRegistryInlineError('');
    try {
      const response = await apiClient.checkSkillRegistryUpdates(teamId);
      setRegistryUpdates(Object.fromEntries(response.updates.map((item) => [item.imported_skill_id, item])));
      setRegistryUpdatesLoaded(true);
      const changed = response.updates.filter((item) => item.has_update).length;
      addToast(
        'success',
        changed > 0
          ? t('teams.resource.skillRegistry.checkUpdatesFound', { count: changed })
          : t('teams.resource.skillRegistry.checkUpdatesNone')
      );
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setCheckingRegistryUpdates(false);
    }
  };

  const handleUpgradeRegistrySkill = async (skill: ImportedRegistrySkillSummary) => {
    setUpgradingRegistrySkillId(skill.imported_skill_id);
    setRegistryInlineError('');
    try {
      const response = await apiClient.upgradeSkillRegistrySkill({
        teamId,
        importedSkillId: skill.imported_skill_id,
      });
      addToast(
        response.upgraded ? 'success' : 'info',
        response.upgraded
          ? t('teams.resource.skillRegistry.upgradeSuccess', { name: response.name })
          : response.reason || t('teams.resource.skillRegistry.checkUpdatesNone')
      );
      await Promise.all([loadSkills(), loadImportedRegistrySkills()]);
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setUpgradingRegistrySkillId(null);
    }
  };

  if (loading) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>;
  }

  if (error) {
    return <p className="text-center py-8 text-[hsl(var(--destructive))]">{error}</p>;
  }

  return (
    <>
      <Card className="ui-section-panel mb-6">
        <CardHeader>
          <CardTitle className="ui-heading text-[22px]">{t('teams.resource.skillRegistry.title')}</CardTitle>
          <CardDescription className="ui-secondary-text">{t('teams.resource.skillRegistry.description')}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="flex items-center gap-2 flex-wrap">
            <div className="relative min-w-[180px] flex-1 md:min-w-[220px]">
              <Search className="absolute left-2 top-2.5 h-4 w-4 text-[hsl(var(--muted-foreground))]" />
              <Input
                className="pl-8"
                placeholder={t('teams.resource.skillRegistry.searchPlaceholder')}
                aria-label={t('teams.resource.skillRegistry.searchPlaceholder')}
                value={registryQuery}
                onChange={(e) => setRegistryQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void handleRegistrySearch();
                }}
              />
            </div>
            <Button onClick={handleRegistrySearch} disabled={registrySearching}>
              <Search className="h-4 w-4 mr-2" />
              {registrySearching ? t('teams.resource.skillRegistry.searching') : t('teams.resource.skillRegistry.searchAction')}
            </Button>
          </div>

          {registryInlineError ? (
            <div className="rounded-[18px] border border-[hsl(var(--destructive))]/30 bg-[hsl(var(--destructive))]/6 px-3 py-2 text-sm text-[hsl(var(--destructive))]">
              {registryInlineError}
            </div>
          ) : null}

          <div className="space-y-3">
            <div>
              <h3 className="text-sm font-medium">{t('teams.resource.skillRegistry.resultsTitle')}</h3>
              <p className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.resultsDescription')}</p>
            </div>
            {registrySearchPerformed && registryResults.length === 0 ? (
              <div className="ui-empty-panel px-4 py-6 text-sm ui-secondary-text">
                {t('teams.resource.skillRegistry.noResults')}
              </div>
            ) : null}
            {registryResults.length > 0 ? (
              <div className="space-y-3">
                {registryResults.map((result) => {
                  const previewKey = `${result.source}:${result.skill_id}`;
                  return (
                    <div key={previewKey} className="ui-subtle-panel flex flex-col gap-3 p-4">
                      <div className="flex items-start justify-between gap-3 flex-wrap">
                        <div className="space-y-1">
                          <div className="text-sm font-semibold text-[hsl(var(--foreground))]">{result.name}</div>
                          <div className="ui-tertiary-text text-xs">
                            {t('teams.resource.skillRegistry.source')}: {result.source}
                          </div>
                        </div>
                        <div className="flex items-center gap-2 flex-wrap">
                          <Badge variant="outline">{t('teams.resource.skillRegistry.installs')}: {result.installs}</Badge>
                          {result.is_duplicate ? <Badge variant="secondary">{t('teams.resource.skillRegistry.duplicate')}</Badge> : null}
                        </div>
                      </div>
                      <div className="flex items-center gap-2 flex-wrap">
                        <Button variant="outline" size="sm" disabled={!result.supports_preview || previewLoadingKey === previewKey} onClick={() => void handleRegistryPreview(result)}>
                          <Eye className={`h-4 w-4 mr-2 ${previewLoadingKey === previewKey ? 'animate-pulse' : ''}`} />
                          {t('teams.resource.skillRegistry.previewAction')}
                        </Button>
                        <Button size="sm" disabled={!result.supports_import || importingRegistryKey === previewKey} onClick={() => void handleRegistryImport(result.source, result.skill_id)}>
                          <Download className="h-4 w-4 mr-2" />
                          {importingRegistryKey === previewKey ? t('teams.resource.skillRegistry.importing') : t('teams.resource.skillRegistry.importAction')}
                        </Button>
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : null}
          </div>

          <div className="ui-section-divider pt-6 space-y-3">
            <div className="flex items-start justify-between gap-3 flex-wrap">
              <div>
                <h3 className="text-sm font-medium">{t('teams.resource.skillRegistry.importedTitle')}</h3>
                <p className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.importedDescription')}</p>
              </div>
              <Button variant="outline" onClick={handleCheckRegistryUpdates} disabled={checkingRegistryUpdates || registryImportedLoading || registryImportedSkills.length === 0}>
                <RefreshCw className={`h-4 w-4 mr-2 ${checkingRegistryUpdates ? 'animate-spin' : ''}`} />
                {t('teams.resource.skillRegistry.checkUpdates')}
              </Button>
            </div>
            {registryImportedLoading ? (
              <p className="ui-secondary-text text-sm">{t('common.loading')}</p>
            ) : registryImportedSkills.length === 0 ? (
              <div className="ui-empty-panel px-4 py-6 text-sm ui-secondary-text">
                {t('teams.resource.skillRegistry.noImported')}
              </div>
            ) : (
              <div className="space-y-3">
                {registryImportedSkills.map((skill) => {
                  const update = registryUpdates[skill.imported_skill_id];
                  const hasUpdate = Boolean(update?.has_update);
                  return (
                    <div key={skill.imported_skill_id} className="ui-subtle-panel flex flex-col gap-3 p-4">
                      <div className="flex items-start justify-between gap-3 flex-wrap">
                        <div className="space-y-1">
                          <div className="text-sm font-semibold text-[hsl(var(--foreground))]">{skill.name}</div>
                          <div className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.source')}: {skill.source}</div>
                          <div className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.sourceRef')}: {skill.source_ref}</div>
                          <div className="ui-tertiary-text text-xs">
                            {t('teams.resource.version')}: {skill.version} · {formatDateTime(skill.updated_at)}
                          </div>
                        </div>
                        <div className="flex items-center gap-2 flex-wrap">
                          {skill.registry_provider ? <Badge variant="outline">{skill.registry_provider}</Badge> : null}
                          {registryUpdatesLoaded ? (
                            <Badge variant={hasUpdate ? 'default' : 'secondary'}>
                              {hasUpdate ? t('teams.resource.skillRegistry.updateAvailable') : t('teams.resource.skillRegistry.upToDate')}
                            </Badge>
                          ) : null}
                          {skill.source_url ? (
                            <a href={skill.source_url} target="_blank" rel="noreferrer" className="ui-inline-action text-xs">
                              {t('teams.resource.skillRegistry.openSource')}
                            </a>
                          ) : null}
                        </div>
                      </div>
                      {skill.description ? <p className="ui-secondary-text text-sm">{skill.description}</p> : null}
                      <div className="flex items-center gap-2 flex-wrap">
                        {canManage ? (
                          <Button size="sm" variant="outline" onClick={() => void handleUpgradeRegistrySkill(skill)} disabled={upgradingRegistrySkillId === skill.imported_skill_id}>
                            <RefreshCw className={`h-4 w-4 mr-2 ${upgradingRegistrySkillId === skill.imported_skill_id ? 'animate-spin' : ''}`} />
                            {t('teams.resource.skillRegistry.upgradeAction')}
                          </Button>
                        ) : null}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      <div className="mb-4 flex items-center gap-2 flex-wrap">
        <div className="relative min-w-[160px] flex-1 sm:min-w-[200px]">
          <Search className="absolute left-2 top-2.5 h-4 w-4 text-[hsl(var(--muted-foreground))]" />
          <Input
            className="pl-8"
            placeholder={t('common.search')}
            aria-label={t('common.search')}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <Select value={sort} onValueChange={setSort}>
          <SelectTrigger className="w-full sm:w-[min(180px,100%)]">
            <SelectValue placeholder={t('teams.resource.sortLabel', 'Sort by')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="updated_at">{t('teams.resource.sortUpdated')}</SelectItem>
            <SelectItem value="name">{t('teams.resource.sortName')}</SelectItem>
            <SelectItem value="created_at">{t('teams.resource.sortCreated')}</SelectItem>
            <SelectItem value="use_count">{t('teams.resource.sortUsage')}</SelectItem>
          </SelectContent>
        </Select>
        {canManage ? (
          <>
            <Button variant="outline" onClick={handleBackfill} disabled={backfilling}>
              <RefreshCw className={`h-4 w-4 mr-2 ${backfilling ? 'animate-spin' : ''}`} />
              {t('teams.resource.backfillMd')}
            </Button>
            <Button onClick={() => setCreateDialogOpen(true)}>
              <Plus className="h-4 w-4 mr-2" />
              {t('teams.resource.createSkill')}
            </Button>
          </>
        ) : null}
      </div>

      {skills.length === 0 ? (
        <div className="ui-empty-panel py-8 text-center text-sm ui-secondary-text">{t('teams.resource.noSkills')}</div>
      ) : (
        <>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t('teams.resource.name')}</TableHead>
                <TableHead>{t('teams.resource.author')}</TableHead>
                <TableHead>{t('teams.resource.version')}</TableHead>
                <TableHead>{t('teams.resource.usageCount')}</TableHead>
                <TableHead className="w-[150px] sm:w-[180px]">{t('common.actions')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {skills.map((skill) => (
                <Fragment key={skill.id}>
                  <TableRow>
                    <TableCell className="font-medium">{skill.name}</TableCell>
                    <TableCell>{skill.authorId}</TableCell>
                    <TableCell>{skill.version}</TableCell>
                    <TableCell>{skill.useCount}</TableCell>
                    <TableCell>
                      <div className="flex gap-1">
                        <Button variant="ghost" size="sm" onClick={() => void handleViewOrEdit(skill, 'view')} disabled={fetchingSkillId === skill.id} title={t('common.view')}>
                          <Eye className={`h-4 w-4 ${fetchingSkillId === skill.id ? 'animate-pulse' : ''}`} />
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => void handleAiDescribe(skill.id)} disabled={describingId === skill.id} title={t('aiInsights.describe')}>
                          <Sparkles className={`h-4 w-4 ${skill.aiDescription ? 'text-status-warning-text' : ''} ${describingId === skill.id ? 'animate-spin' : ''}`} />
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => void handleInstall(skill.id)} disabled={installingId === skill.id} title={t('teams.resource.install')}>
                          <Download className="h-4 w-4" />
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setConfirmAction({ type: 'uninstall', id: skill.id })} title={t('teams.resource.uninstall')}>
                          <X className="h-4 w-4" />
                        </Button>
                        <Button variant="ghost" size="sm" onClick={() => setAddToAgentSkill({ id: skill.id, name: skill.name })} title={t('agent.skills.addSkillToAgent')}>
                          <Bot className="h-4 w-4" />
                        </Button>
                        {canManage ? (
                          <>
                            <Button variant="ghost" size="sm" onClick={() => void handleViewOrEdit(skill, 'edit')} disabled={fetchingSkillId === skill.id} title={t('common.edit')}>
                              <Pencil className="h-4 w-4" />
                            </Button>
                            <Button variant="ghost" size="sm" onClick={() => setConfirmAction({ type: 'delete', id: skill.id })} title={t('common.delete')}>
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </>
                        ) : null}
                      </div>
                    </TableCell>
                  </TableRow>
                  {expandedDescriptions.has(skill.id) && skill.aiDescription ? (
                    <TableRow>
                      <TableCell colSpan={5} className="bg-[hsl(var(--ui-surface-panel-muted))/0.76] p-4">
                        <div className="text-sm whitespace-pre-wrap">{skill.aiDescription}</div>
                        {skill.aiDescribedAt ? (
                          <div className="ui-tertiary-text mt-2 text-xs">
                            {t('aiInsights.generatedAt')}: {formatDateTime(skill.aiDescribedAt)}
                          </div>
                        ) : null}
                      </TableCell>
                    </TableRow>
                  ) : null}
                </Fragment>
              ))}
            </TableBody>
          </Table>
          {totalPages > 1 ? (
            <div className="flex items-center justify-between mt-4">
              <span className="text-sm text-[hsl(var(--muted-foreground))]">{t('common.total')}: {total}</span>
              <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage((value) => value - 1)}>
                  <ChevronLeft className="h-4 w-4" />
                </Button>
                <span className="text-sm">{page} / {totalPages}</span>
                <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage((value) => value + 1)}>
                  <ChevronRight className="h-4 w-4" />
                </Button>
              </div>
            </div>
          ) : null}
        </>
      )}

      <Dialog open={!!previewSkill} onOpenChange={(open) => { if (!open) setPreviewSkill(null); }}>
        <DialogContent className="max-w-4xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{previewSkill?.name || t('teams.resource.skillRegistry.previewTitle')}</DialogTitle>
            <DialogDescription>{t('teams.resource.skillRegistry.previewDescription')}</DialogDescription>
          </DialogHeader>
          {previewSkill ? (
            <div className="space-y-4">
              <div className="grid gap-3 md:grid-cols-2">
                <div className="ui-subtle-panel p-3 space-y-2">
                  <div className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.source')}: {previewSkill.source}</div>
                  <div className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.sourceRef')}: {previewSkill.source_ref}</div>
                  <div className="ui-tertiary-text text-xs">Commit: {previewSkill.source_commit}</div>
                  <div className="ui-tertiary-text text-xs">Path: {previewSkill.skill_dir}</div>
                  <div className="flex items-center gap-2 flex-wrap">
                    {previewSkill.already_imported ? <Badge variant="secondary">{t('teams.resource.skillRegistry.alreadyImported')}</Badge> : null}
                    {previewSkill.tags.map((tag) => (
                      <Badge key={tag} variant="outline">{tag}</Badge>
                    ))}
                  </div>
                </div>
                <div className="ui-subtle-panel p-3 space-y-2">
                  <div className="text-sm font-medium">{t('teams.resource.skillRegistry.remoteFiles')}</div>
                  {previewSkill.files.length === 0 ? (
                    <div className="ui-tertiary-text text-xs">{t('teams.resource.noFiles')}</div>
                  ) : (
                    <div className="max-h-40 overflow-y-auto space-y-1">
                      {previewSkill.files.map((file) => (
                        <div key={file.path} className="ui-tertiary-text text-xs">{file.path}</div>
                      ))}
                    </div>
                  )}
                  {previewSkill.skipped_files.length > 0 ? (
                    <div className="space-y-1">
                      <div className="text-xs font-medium">{t('teams.resource.skillRegistry.skippedFiles')}</div>
                      {previewSkill.skipped_files.map((file) => (
                        <div key={file} className="ui-tertiary-text text-xs">{file}</div>
                      ))}
                    </div>
                  ) : null}
                </div>
              </div>
              <div className="space-y-2">
                <div className="text-sm font-medium">{t('teams.resource.skillRegistry.skillMdPreview')}</div>
                <Textarea value={previewSkill.skill_md} readOnly className="min-h-[320px] font-mono text-xs" />
                {previewSkill.truncated ? (
                  <p className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.previewTruncated')}</p>
                ) : null}
              </div>
            </div>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setPreviewSkill(null)}>{t('common.close')}</Button>
            {previewSkill ? (
              <Button
                disabled={previewSkill.already_imported || importingRegistryKey === `${previewSkill.source}:${previewSkill.skill_id}`}
                onClick={() => void handleRegistryImport(previewSkill.source, previewSkill.skill_id, previewSkill.source_ref)}
              >
                <Download className="h-4 w-4 mr-2" />
                {importingRegistryKey === `${previewSkill.source}:${previewSkill.skill_id}`
                  ? t('teams.resource.skillRegistry.importing')
                  : t('teams.resource.skillRegistry.importAction')}
              </Button>
            ) : null}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ResourceDetailDialog
        open={!!selectedSkill}
        onOpenChange={() => setSelectedSkill(null)}
        resource={selectedSkill}
        resourceType="skill"
        mode={dialogMode}
        onSave={async (data) => {
          if (selectedSkill) {
            await apiClient.updateSkill(selectedSkill.id, data);
            loadSkills();
          }
        }}
      />

      <CreateSkillDialog
        teamId={teamId}
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={loadSkills}
      />

      {addToAgentSkill ? (
        <AddSkillToAgentDialog
          open={!!addToAgentSkill}
          onOpenChange={(open) => {
            if (!open) setAddToAgentSkill(null);
          }}
          skillId={addToAgentSkill.id}
          skillName={addToAgentSkill.name}
          teamId={teamId}
        />
      ) : null}

      <ConfirmDialog
        open={!!confirmAction}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
        title={t(confirmAction?.type === 'delete' ? 'teams.resource.deleteConfirm' : 'teams.resource.uninstallConfirm')}
        variant="destructive"
        onConfirm={handleConfirm}
      />
    </>
  );
}
