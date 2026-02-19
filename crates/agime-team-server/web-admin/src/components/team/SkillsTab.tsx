import { Fragment, useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useToast } from '../../contexts/ToastContext';
import { Eye, Pencil, Trash2, Plus, Download, X, Search, ChevronLeft, ChevronRight, Bot, Sparkles, RefreshCw } from 'lucide-react';
import { Button } from '../ui/button';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../ui/table';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { ResourceDetailDialog } from './ResourceDetailDialog';
import { CreateSkillDialog } from './CreateSkillDialog';
import { AddSkillToAgentDialog } from './AddSkillToAgentDialog';
import { apiClient } from '../../api/client';
import type { SharedSkill } from '../../api/types';

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

  function getErrorMsg(err: unknown, fallbackKey = 'common.error'): string {
    return err instanceof Error ? err.message : t(fallbackKey);
  }

  const loadSkills = useCallback(async () => {
    try {
      setLoading(true);
      const response = await apiClient.getSkills(teamId, {
        page, limit: 20, search: search || undefined, sort,
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

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  // Reset all UI state when teamId changes to prevent cross-team data leaks
  useEffect(() => {
    setSearch('');
    setSort('updated_at');
    setPage(1);
    setSelectedSkill(null);
    setAddToAgentSkill(null);
    setExpandedDescriptions(new Set());
    setError('');
  }, [teamId]);

  useEffect(() => { setPage(1); }, [search, sort]);

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
    const skill = skills.find(s => s.id === skillId);
    if (skill?.aiDescription && skill.aiDescriptionLang === i18n.language.substring(0, 2)) {
      setExpandedDescriptions(prev => {
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
      setSkills(prev => prev.map(s =>
        s.id === skillId ? { ...s, aiDescription: result.description, aiDescriptionLang: result.lang, aiDescribedAt: result.generated_at } : s
      ));
      setExpandedDescriptions(prev => new Set(prev).add(skillId));
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

  if (loading) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>;
  }

  if (error) {
    return <p className="text-center py-8 text-[hsl(var(--destructive))]">{error}</p>;
  }

  return (
    <>
      <div className="mb-4 flex items-center gap-2 flex-wrap">
        <div className="relative flex-1 min-w-[200px]">
          <Search className="absolute left-2 top-2.5 h-4 w-4 text-[hsl(var(--muted-foreground))]" />
          <input
            className="w-full pl-8 pr-3 py-2 rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] text-sm"
            placeholder={t('common.search')}
            aria-label={t('common.search')}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <select
          className="px-3 py-2 rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] text-sm"
          value={sort}
          aria-label={t('teams.resource.sortLabel', 'Sort by')}
          onChange={(e) => setSort(e.target.value)}
        >
          <option value="updated_at">{t('teams.resource.sortUpdated')}</option>
          <option value="name">{t('teams.resource.sortName')}</option>
          <option value="created_at">{t('teams.resource.sortCreated')}</option>
          <option value="use_count">{t('teams.resource.sortUsage')}</option>
        </select>
        {canManage && (
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
        )}
      </div>

      {skills.length === 0 ? (
        <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.resource.noSkills')}</p>
      ) : (
      <>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t('teams.resource.name')}</TableHead>
            <TableHead>{t('teams.resource.author')}</TableHead>
            <TableHead>{t('teams.resource.version')}</TableHead>
            <TableHead>{t('teams.resource.usageCount')}</TableHead>
            <TableHead className="w-[180px]">{t('common.actions')}</TableHead>
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
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleViewOrEdit(skill, 'view')}
                    disabled={fetchingSkillId === skill.id}
                    title={t('common.view')}
                  >
                    <Eye className={`h-4 w-4 ${fetchingSkillId === skill.id ? 'animate-pulse' : ''}`} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleAiDescribe(skill.id)}
                    disabled={describingId === skill.id}
                    title={t('aiInsights.describe')}
                  >
                    <Sparkles className={`h-4 w-4 ${skill.aiDescription ? 'text-amber-500' : ''} ${describingId === skill.id ? 'animate-spin' : ''}`} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleInstall(skill.id)}
                    disabled={installingId === skill.id}
                    title={t('teams.resource.install')}
                  >
                    <Download className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setConfirmAction({ type: 'uninstall', id: skill.id })}
                    title={t('teams.resource.uninstall')}
                  >
                    <X className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setAddToAgentSkill({ id: skill.id, name: skill.name })}
                    title={t('agent.skills.addSkillToAgent')}
                  >
                    <Bot className="h-4 w-4" />
                  </Button>
                  {canManage && (
                    <>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleViewOrEdit(skill, 'edit')}
                        disabled={fetchingSkillId === skill.id}
                        title={t('common.edit')}
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setConfirmAction({ type: 'delete', id: skill.id })}
                        title={t('common.delete')}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </>
                  )}
                </div>
              </TableCell>
            </TableRow>
            {expandedDescriptions.has(skill.id) && skill.aiDescription && (
              <TableRow>
                <TableCell colSpan={5} className="bg-[hsl(var(--muted))] p-4">
                  <div className="text-sm whitespace-pre-wrap">{skill.aiDescription}</div>
                  {skill.aiDescribedAt && (
                    <div className="text-xs text-[hsl(var(--muted-foreground))] mt-2">
                      {t('aiInsights.generatedAt')}: {new Date(skill.aiDescribedAt).toLocaleString()}
                    </div>
                  )}
                </TableCell>
              </TableRow>
            )}
            </Fragment>
          ))}
        </TableBody>
      </Table>

      {totalPages > 1 && (
        <div className="flex items-center justify-between mt-4">
          <span className="text-sm text-[hsl(var(--muted-foreground))]">
            {t('common.total')}: {total}
          </span>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage(p => p - 1)}>
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <span className="text-sm">{page} / {totalPages}</span>
            <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage(p => p + 1)}>
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      )}
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

      {addToAgentSkill && (
        <AddSkillToAgentDialog
          open={!!addToAgentSkill}
          onOpenChange={(open) => { if (!open) setAddToAgentSkill(null); }}
          skillId={addToAgentSkill.id}
          skillName={addToAgentSkill.name}
          teamId={teamId}
        />
      )}

      <ConfirmDialog
        open={!!confirmAction}
        onOpenChange={(open) => { if (!open) setConfirmAction(null); }}
        title={t(confirmAction?.type === 'delete' ? 'teams.resource.deleteConfirm' : 'teams.resource.uninstallConfirm')}
        variant="destructive"
        onConfirm={handleConfirm}
      />
    </>
  );
}
