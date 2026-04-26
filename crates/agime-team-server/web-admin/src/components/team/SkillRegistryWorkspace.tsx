import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Download, Eye, RefreshCw, Search } from 'lucide-react';
import { useToast } from '../../contexts/ToastContext';
import { apiClient } from '../../api/client';
import type {
  ImportedRegistrySkillSummary,
  SkillRegistryPreviewResponse,
  SkillRegistrySearchItem,
  SkillRegistryUpdateInspection,
} from '../../api/types';
import { formatDateTime } from '../../utils/format';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../ui/card';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';

interface SkillRegistryWorkspaceProps {
  teamId: string;
  canManage: boolean;
  compact?: boolean;
}

export function SkillRegistryWorkspace({
  teamId,
  canManage,
  compact = false,
}: SkillRegistryWorkspaceProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [registryQuery, setRegistryQuery] = useState('');
  const [directInstallSpec, setDirectInstallSpec] = useState('');
  const [registrySearching, setRegistrySearching] = useState(false);
  const [registrySearchPerformed, setRegistrySearchPerformed] = useState(false);
  const [registryResults, setRegistryResults] = useState<SkillRegistrySearchItem[]>([]);
  const [registryImportedSkills, setRegistryImportedSkills] = useState<ImportedRegistrySkillSummary[]>([]);
  const [registryImportedLoading, setRegistryImportedLoading] = useState(true);
  const [registryInlineError, setRegistryInlineError] = useState('');
  const [previewLoadingKey, setPreviewLoadingKey] = useState<string | null>(null);
  const [previewSkill, setPreviewSkill] = useState<SkillRegistryPreviewResponse | null>(null);
  const [importingRegistryKey, setImportingRegistryKey] = useState<string | null>(null);
  const [directSpecAction, setDirectSpecAction] = useState<'preview' | 'import' | null>(null);
  const [checkingRegistryUpdates, setCheckingRegistryUpdates] = useState(false);
  const [registryUpdatesLoaded, setRegistryUpdatesLoaded] = useState(false);
  const [registryUpdates, setRegistryUpdates] = useState<Record<string, SkillRegistryUpdateInspection>>({});
  const [upgradingRegistrySkillId, setUpgradingRegistrySkillId] = useState<string | null>(null);

  const getErrorMsg = useCallback((err: unknown, fallbackKey = 'common.error') => (
    err instanceof Error ? err.message : t(fallbackKey)
  ), [t]);

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
  }, [getErrorMsg, teamId]);

  useEffect(() => {
    void loadImportedRegistrySkills();
  }, [loadImportedRegistrySkills]);

  useEffect(() => {
    setRegistryQuery('');
    setRegistryResults([]);
    setRegistrySearchPerformed(false);
    setPreviewSkill(null);
    setRegistryInlineError('');
    setRegistryUpdates({});
    setRegistryUpdatesLoaded(false);
  }, [teamId]);

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

  const handleDirectRegistryPreview = async () => {
    const installSpec = directInstallSpec.trim();
    if (!installSpec) return;
    setDirectSpecAction('preview');
    setRegistryInlineError('');
    try {
      const response = await apiClient.previewSkillRegistrySkill({
        teamId,
        installSpec,
      });
      setPreviewSkill(response);
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setDirectSpecAction(null);
    }
  };

  const handleDirectRegistryImport = async () => {
    const installSpec = directInstallSpec.trim();
    if (!installSpec) return;
    setDirectSpecAction('import');
    setRegistryInlineError('');
    try {
      const response = await apiClient.importSkillRegistrySkill({
        teamId,
        installSpec,
      });
      addToast('success', t('teams.resource.skillRegistry.importSuccess', { name: response.name }));
      await loadImportedRegistrySkills();
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setDirectSpecAction(null);
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
      await loadImportedRegistrySkills();
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
          : t('teams.resource.skillRegistry.checkUpdatesNone'),
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
          : response.reason || t('teams.resource.skillRegistry.checkUpdatesNone'),
      );
      await loadImportedRegistrySkills();
    } catch (err) {
      setRegistryInlineError(getErrorMsg(err));
    } finally {
      setUpgradingRegistrySkillId(null);
    }
  };

  return (
    <>
      <Card className={compact ? 'ui-section-panel' : 'ui-section-panel'}>
        <CardHeader className={compact ? 'pb-4' : undefined}>
          <CardTitle className={`ui-heading ${compact ? 'text-[20px]' : 'text-[24px]'}`}>
            {t('teams.resource.skillRegistry.title')}
          </CardTitle>
          <CardDescription className="ui-secondary-text">
            {t('teams.resource.skillRegistry.description')}
          </CardDescription>
        </CardHeader>
        <CardContent className={compact ? 'space-y-5' : 'space-y-6'}>
          <div className="flex items-center gap-2 flex-wrap">
            <div className="relative min-w-[180px] flex-1 md:min-w-[280px]">
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

          <div className="ui-subtle-panel p-4 space-y-3">
            <div>
              <h3 className="text-sm font-medium">
                {t('teams.resource.skillRegistry.directInstallTitle', 'Install by spec or URL')}
              </h3>
              <p className="ui-tertiary-text text-xs">
                {t(
                  'teams.resource.skillRegistry.directInstallDescription',
                  'Paste owner/repo@skill, a skills.sh URL, or a GitHub repo/tree URL. AGIME imports into the team skill library, not the local filesystem.',
                )}
              </p>
            </div>
            <div className="flex items-center gap-2 flex-wrap">
              <Input
                className="min-w-[260px] flex-1"
                placeholder="vercel-labs/skills@find-skills"
                value={directInstallSpec}
                onChange={(e) => setDirectInstallSpec(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void handleDirectRegistryPreview();
                }}
              />
              <Button
                variant="outline"
                onClick={() => void handleDirectRegistryPreview()}
                disabled={!directInstallSpec.trim() || directSpecAction !== null}
              >
                <Eye className="h-4 w-4 mr-2" />
                {directSpecAction === 'preview'
                  ? t('common.loading')
                  : t('teams.resource.skillRegistry.previewAction')}
              </Button>
              <Button
                onClick={() => void handleDirectRegistryImport()}
                disabled={!directInstallSpec.trim() || directSpecAction !== null}
              >
                <Download className="h-4 w-4 mr-2" />
                {directSpecAction === 'import'
                  ? t('teams.resource.skillRegistry.importing')
                  : t('teams.resource.skillRegistry.importAction')}
              </Button>
            </div>
          </div>

          {registryInlineError ? (
            <div className="rounded-[18px] border border-[hsl(var(--destructive))]/30 bg-[hsl(var(--destructive))]/6 px-3 py-2 text-sm text-[hsl(var(--destructive))]">
              {registryInlineError}
            </div>
          ) : null}

          <section className="space-y-3">
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
                          {result.install_spec ? (
                            <div className="ui-tertiary-text text-xs font-mono">
                              {result.install_spec}
                            </div>
                          ) : null}
                        </div>
                        <div className="flex items-center gap-2 flex-wrap">
                          <Badge variant="outline">{t('teams.resource.skillRegistry.installs')}: {result.installs}</Badge>
                          {result.is_duplicate ? <Badge variant="secondary">{t('teams.resource.skillRegistry.duplicate')}</Badge> : null}
                        </div>
                      </div>
                      <div className="flex items-center gap-2 flex-wrap">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={!result.supports_preview || previewLoadingKey === previewKey}
                          onClick={() => void handleRegistryPreview(result)}
                        >
                          <Eye className={`h-4 w-4 mr-2 ${previewLoadingKey === previewKey ? 'animate-pulse' : ''}`} />
                          {t('teams.resource.skillRegistry.previewAction')}
                        </Button>
                        <Button
                          size="sm"
                          disabled={!result.supports_import || importingRegistryKey === previewKey}
                          onClick={() => void handleRegistryImport(result.source, result.skill_id)}
                        >
                          <Download className="h-4 w-4 mr-2" />
                          {importingRegistryKey === previewKey ? t('teams.resource.skillRegistry.importing') : t('teams.resource.skillRegistry.importAction')}
                        </Button>
                      </div>
                    </div>
                  );
                })}
              </div>
            ) : null}
          </section>

          <section className="ui-section-divider pt-6 space-y-3">
            <div className="flex items-start justify-between gap-3 flex-wrap">
              <div>
                <h3 className="text-sm font-medium">{t('teams.resource.skillRegistry.importedTitle')}</h3>
                <p className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.importedDescription')}</p>
              </div>
              <Button
                variant="outline"
                onClick={handleCheckRegistryUpdates}
                disabled={checkingRegistryUpdates || registryImportedLoading || registryImportedSkills.length === 0}
              >
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
                      {canManage ? (
                        <div className="flex items-center gap-2 flex-wrap">
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => void handleUpgradeRegistrySkill(skill)}
                            disabled={upgradingRegistrySkillId === skill.imported_skill_id}
                          >
                            <RefreshCw className={`h-4 w-4 mr-2 ${upgradingRegistrySkillId === skill.imported_skill_id ? 'animate-spin' : ''}`} />
                            {t('teams.resource.skillRegistry.upgradeAction')}
                          </Button>
                        </div>
                      ) : null}
                    </div>
                  );
                })}
              </div>
            )}
          </section>
        </CardContent>
      </Card>

      <Dialog open={!!previewSkill} onOpenChange={(open) => { if (!open) setPreviewSkill(null); }}>
        <DialogContent className="max-w-4xl max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{previewSkill?.name || t('teams.resource.skillRegistry.previewTitle')}</DialogTitle>
            <DialogDescription>{t('teams.resource.skillRegistry.previewDescription')}</DialogDescription>
          </DialogHeader>
          {previewSkill ? (
            <div className="space-y-4">
              {previewSkill.resolution_status === 'multiple_candidates' ? (
                <div className="space-y-3">
                  <div className="ui-subtle-panel p-3">
                    <div className="text-sm font-medium">
                      {t('teams.resource.skillRegistry.multipleCandidatesTitle', 'Multiple skills found')}
                    </div>
                    <p className="ui-secondary-text text-sm">
                      {previewSkill.message || 'Choose one candidate and preview or import its install spec.'}
                    </p>
                  </div>
                  {(previewSkill.candidates || []).map((candidate) => (
                    <div key={candidate.install_spec} className="ui-subtle-panel flex flex-col gap-2 p-3">
                      <div className="text-sm font-semibold">{candidate.name}</div>
                      <div className="ui-tertiary-text text-xs">{candidate.skill_dir}</div>
                      <div className="ui-tertiary-text text-xs font-mono">{candidate.install_spec}</div>
                      <div className="flex items-center gap-2 flex-wrap">
                        <Button
                          size="sm"
                          variant="outline"
                          onClick={() => {
                            setDirectInstallSpec(candidate.install_spec);
                            setPreviewSkill(null);
                            void apiClient.previewSkillRegistrySkill({
                              teamId,
                              installSpec: candidate.install_spec,
                            }).then(setPreviewSkill).catch((err) => setRegistryInlineError(getErrorMsg(err)));
                          }}
                        >
                          <Eye className="h-4 w-4 mr-2" />
                          {t('teams.resource.skillRegistry.previewAction')}
                        </Button>
                        <Button
                          size="sm"
                          onClick={() => {
                            setPreviewSkill(null);
                            setDirectInstallSpec(candidate.install_spec);
                            void handleRegistryImport(candidate.source, candidate.skill_id, candidate.source_ref);
                          }}
                        >
                          <Download className="h-4 w-4 mr-2" />
                          {t('teams.resource.skillRegistry.importAction')}
                        </Button>
                      </div>
                    </div>
                  ))}
                </div>
              ) : previewSkill.resolution_status === 'not_found' ? (
                <div className="ui-empty-panel px-4 py-6 text-sm ui-secondary-text">
                  {previewSkill.message || t('teams.resource.skillRegistry.noResults')}
                </div>
              ) : (
                <>
              <div className="grid gap-3 md:grid-cols-2">
                <div className="ui-subtle-panel p-3 space-y-2">
                  <div className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.source')}: {previewSkill.source}</div>
                  <div className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.sourceRef')}: {previewSkill.source_ref}</div>
                  <div className="ui-tertiary-text text-xs">Commit: {previewSkill.source_commit}</div>
                  <div className="ui-tertiary-text text-xs">Path: {previewSkill.skill_dir}</div>
                  {previewSkill.install_spec ? (
                    <div className="ui-tertiary-text text-xs font-mono">{previewSkill.install_spec}</div>
                  ) : null}
                  <div className="flex items-center gap-2 flex-wrap">
                    {previewSkill.already_imported ? <Badge variant="secondary">{t('teams.resource.skillRegistry.alreadyImported')}</Badge> : null}
                    {(previewSkill.tags || []).map((tag) => (
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
                <Textarea value={previewSkill.skill_md || ''} readOnly className="min-h-[320px] font-mono text-xs" />
                {previewSkill.truncated ? (
                  <p className="ui-tertiary-text text-xs">{t('teams.resource.skillRegistry.previewTruncated')}</p>
                ) : null}
              </div>
                </>
              )}
            </div>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setPreviewSkill(null)}>{t('common.close')}</Button>
            {previewSkill ? (
              <Button
                disabled={
                  previewSkill.resolution_status !== 'resolved'
                    || previewSkill.already_imported
                    || importingRegistryKey === `${previewSkill.source}:${previewSkill.skill_id}`
                }
                onClick={() => {
                  if (previewSkill.install_spec) {
                    setDirectInstallSpec(previewSkill.install_spec);
                    void apiClient.importSkillRegistrySkill({
                      teamId,
                      installSpec: previewSkill.install_spec,
                    }).then(async (response) => {
                      addToast('success', t('teams.resource.skillRegistry.importSuccess', { name: response.name }));
                      await loadImportedRegistrySkills();
                      setPreviewSkill((prev) => prev ? { ...prev, already_imported: true } : prev);
                    }).catch((err) => setRegistryInlineError(getErrorMsg(err)));
                    return;
                  }
                  if (previewSkill.skill_id) {
                    void handleRegistryImport(previewSkill.source, previewSkill.skill_id, previewSkill.source_ref);
                  }
                }}
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
    </>
  );
}
