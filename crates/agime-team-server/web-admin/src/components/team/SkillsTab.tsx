import { Fragment, useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Bot,
  ChevronLeft,
  ChevronRight,
  Download,
  Eye,
  ExternalLink,
  LibraryBig,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Sparkles,
  Trash2,
  Wand2,
  X,
} from 'lucide-react';
import { useToast } from '../../contexts/ToastContext';
import { apiClient } from '../../api/client';
import type { SharedSkill } from '../../api/types';
import { formatDateTime } from '../../utils/format';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '../ui/table';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { ResourceDetailDialog } from './ResourceDetailDialog';
import { CreateSkillDialog } from './CreateSkillDialog';
import { AddSkillToAgentDialog } from './AddSkillToAgentDialog';

type ConfirmAction = { type: 'delete' | 'uninstall'; id: string };

interface SkillsTabProps {
  teamId: string;
  canManage: boolean;
}

export function SkillsTab({ teamId, canManage }: SkillsTabProps) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
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

  const getErrorMsg = useCallback((err: unknown, fallbackKey = 'common.error') => (
    err instanceof Error ? err.message : t(fallbackKey)
  ), [t]);

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
  }, [getErrorMsg, page, search, sort, teamId]);

  useEffect(() => {
    void loadSkills();
  }, [loadSkills]);

  useEffect(() => {
    setSearch('');
    setSort('updated_at');
    setPage(1);
    setSelectedSkill(null);
    setAddToAgentSkill(null);
    setExpandedDescriptions(new Set());
    setError('');
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
      await loadSkills();
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
      await loadSkills();
    } catch (err) {
      setError(getErrorMsg(err));
    } finally {
      setBackfilling(false);
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
      <div className="mb-4 flex items-center gap-2 flex-wrap">
        <div className="relative min-w-[180px] flex-1 sm:min-w-[220px]">
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

      <section className="mb-5 overflow-hidden rounded-[24px] border border-[hsl(var(--ui-line-soft))]/70 bg-[linear-gradient(135deg,hsl(var(--ui-surface-panel-strong))_0%,hsl(var(--ui-surface-panel-muted))_100%)] shadow-[0_16px_36px_hsl(var(--ui-shadow)/0.06)]">
        <div className="grid gap-4 px-5 py-4 lg:grid-cols-[minmax(0,1.4fr)_auto] lg:items-center">
          <div className="min-w-0 space-y-3">
            <div className="flex items-center gap-2">
              <div className="flex h-9 w-9 items-center justify-center rounded-full bg-[hsl(var(--semantic-extension))]/10 text-[hsl(var(--semantic-extension))]">
                <LibraryBig className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                  {t('teams.resource.skillRegistry.launchEyebrow', 'Remote Skills')}
                </div>
                <div className="text-[15px] font-semibold tracking-[-0.02em] text-[hsl(var(--foreground))]">
                  {t('teams.resource.skillRegistry.launchTitle', 'Skill Registry 工作页')}
                </div>
              </div>
            </div>
            <p className="max-w-3xl text-sm leading-6 text-[hsl(var(--ui-text-secondary))]">
              {t(
                'teams.resource.skillRegistry.launchDescription',
                '搜索 skills.sh 远程技能、预览内容，并导入到当前团队技能库。',
              )}
            </p>
            <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs font-medium text-[hsl(var(--ui-text-secondary))]">
              <span className="inline-flex items-center gap-2">
                <span className="h-1.5 w-1.5 rounded-full bg-[hsl(var(--semantic-extension))]" />
                {t('teams.resource.skillRegistry.launchFeatureSearch', '搜索远程技能源')}
              </span>
              <span className="inline-flex items-center gap-2">
                <span className="h-1.5 w-1.5 rounded-full bg-[hsl(var(--semantic-extension))]" />
                {t('teams.resource.skillRegistry.launchFeaturePreview', '预览 SKILL.md 与文件内容')}
              </span>
              <span className="inline-flex items-center gap-2">
                <span className="h-1.5 w-1.5 rounded-full bg-[hsl(var(--semantic-extension))]" />
                {t('teams.resource.skillRegistry.launchFeatureImport', '导入到当前团队技能库')}
              </span>
            </div>
          </div>
          <div className="flex shrink-0 flex-col items-stretch gap-2 sm:flex-row lg:flex-col lg:items-end">
            <Button
              className="min-w-[220px] border-[hsl(var(--semantic-extension))]/24 bg-[hsl(var(--semantic-extension))]/10 text-[hsl(var(--semantic-extension))] shadow-none hover:bg-[hsl(var(--semantic-extension))]/16"
              onClick={() => navigate(`/teams/${teamId}/skills/registry`)}
            >
              <Wand2 className="mr-2 h-4 w-4" />
              {t('teams.resource.skillRegistry.openPage', '打开 Skill Registry')}
            </Button>
            <a
              className="ui-inline-action justify-center lg:justify-end"
              href="https://skills.sh"
              target="_blank"
              rel="noreferrer"
            >
              <ExternalLink className="h-3.5 w-3.5" />
              skills.sh
            </a>
          </div>
        </div>
      </section>

      {skills.length === 0 ? (
        <div className="ui-empty-panel py-8 text-center text-sm ui-secondary-text">{t('teams.resource.noSkills')}</div>
      ) : (
        <>
          <div className="space-y-3 md:hidden">
            {skills.map((skill) => (
              <div
                key={skill.id}
                className="rounded-[22px] border border-[hsl(var(--ui-line-soft))]/80 bg-[hsl(var(--ui-surface-panel))] p-4 shadow-[0_10px_24px_hsl(var(--ui-shadow)/0.05)]"
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-[15px] font-semibold text-[hsl(var(--foreground))]">
                      {skill.name}
                    </div>
                    <div className="mt-1 text-xs text-[hsl(var(--ui-text-secondary))]">
                      {skill.authorId}
                    </div>
                  </div>
                  <div className="shrink-0 rounded-full border border-[hsl(var(--ui-line-soft))] px-2.5 py-1 text-[11px] font-medium text-[hsl(var(--ui-text-secondary))]">
                    v{skill.version}
                  </div>
                </div>

                <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
                  <div className="rounded-2xl bg-[hsl(var(--ui-surface-panel-muted))] px-3 py-2">
                    <div className="text-[hsl(var(--ui-text-tertiary))]">{t('teams.resource.version')}</div>
                    <div className="mt-1 font-medium text-[hsl(var(--foreground))]">{skill.version}</div>
                  </div>
                  <div className="rounded-2xl bg-[hsl(var(--ui-surface-panel-muted))] px-3 py-2">
                    <div className="text-[hsl(var(--ui-text-tertiary))]">{t('teams.resource.usageCount')}</div>
                    <div className="mt-1 font-medium text-[hsl(var(--foreground))]">{skill.useCount}</div>
                  </div>
                </div>

                <div className="mt-4 flex flex-wrap gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => void handleViewOrEdit(skill, 'view')}
                    disabled={fetchingSkillId === skill.id}
                  >
                    <Eye className={`mr-1.5 h-4 w-4 ${fetchingSkillId === skill.id ? 'animate-pulse' : ''}`} />
                    {t('common.view')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => void handleAiDescribe(skill.id)}
                    disabled={describingId === skill.id}
                  >
                    <Sparkles className={`mr-1.5 h-4 w-4 ${skill.aiDescription ? 'text-status-warning-text' : ''} ${describingId === skill.id ? 'animate-spin' : ''}`} />
                    {t('aiInsights.describe')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => void handleInstall(skill.id)}
                    disabled={installingId === skill.id}
                  >
                    <Download className="mr-1.5 h-4 w-4" />
                    {t('teams.resource.install')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => setConfirmAction({ type: 'uninstall', id: skill.id })}
                  >
                    <X className="mr-1.5 h-4 w-4" />
                    {t('teams.resource.uninstall')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => setAddToAgentSkill({ id: skill.id, name: skill.name })}
                  >
                    <Bot className="mr-1.5 h-4 w-4" />
                    {t('agent.skills.addSkillToAgent')}
                  </Button>
                  {canManage ? (
                    <>
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-9 rounded-full px-3"
                        onClick={() => void handleViewOrEdit(skill, 'edit')}
                        disabled={fetchingSkillId === skill.id}
                      >
                        <Pencil className="mr-1.5 h-4 w-4" />
                        {t('common.edit')}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-9 rounded-full px-3 text-[hsl(var(--destructive))] hover:text-[hsl(var(--destructive))]"
                        onClick={() => setConfirmAction({ type: 'delete', id: skill.id })}
                      >
                        <Trash2 className="mr-1.5 h-4 w-4" />
                        {t('common.delete')}
                      </Button>
                    </>
                  ) : null}
                </div>

                {expandedDescriptions.has(skill.id) && skill.aiDescription ? (
                  <div className="mt-4 rounded-2xl bg-[hsl(var(--ui-surface-panel-muted))] px-3 py-3">
                    <div className="text-sm whitespace-pre-wrap text-[hsl(var(--foreground))]">{skill.aiDescription}</div>
                    {skill.aiDescribedAt ? (
                      <div className="ui-tertiary-text mt-2 text-xs">
                        {t('aiInsights.generatedAt')}: {formatDateTime(skill.aiDescribedAt)}
                      </div>
                    ) : null}
                  </div>
                ) : null}
              </div>
            ))}
          </div>

          <div className="hidden md:block">
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
          </div>
          {totalPages > 1 ? (
            <div className="mt-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
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

      <ResourceDetailDialog
        open={!!selectedSkill}
        onOpenChange={() => setSelectedSkill(null)}
        resource={selectedSkill}
        resourceType="skill"
        mode={dialogMode}
        onSave={async (data) => {
          if (selectedSkill) {
            await apiClient.updateSkill(selectedSkill.id, data);
            await loadSkills();
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
