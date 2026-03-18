import { Fragment, useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useToast } from '../../contexts/ToastContext';
import { Eye, Pencil, Trash2, Plus, Download, X, Search, ChevronLeft, ChevronRight, ShieldCheck, Bot, Sparkles, Link2, Info } from 'lucide-react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
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
import { CreateExtensionDialog } from './CreateExtensionDialog';
import { AddExtensionToAgentDialog } from './AddExtensionToAgentDialog';
import { apiClient } from '../../api/client';
import type { SharedExtension } from '../../api/types';
import { formatDateTime } from '../../utils/format';

type ConfirmAction = { type: 'delete' | 'uninstall'; id: string } | null;

interface ExtensionsTabProps {
  teamId: string;
  canManage: boolean;
}

export function ExtensionsTab({ teamId, canManage }: ExtensionsTabProps) {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const { addToast } = useToast();
  const [extensions, setExtensions] = useState<SharedExtension[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedExt, setSelectedExt] = useState<SharedExtension | null>(null);
  const [dialogMode, setDialogMode] = useState<'view' | 'edit'>('view');
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [addToAgentExt, setAddToAgentExt] = useState<SharedExtension | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState('updated_at');
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(1);
  const [total, setTotal] = useState(0);
  const [describingId, setDescribingId] = useState<string | null>(null);
  const [expandedDescriptions, setExpandedDescriptions] = useState<Set<string>>(new Set());
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);
  const [autoAssignExt, setAutoAssignExt] = useState<SharedExtension | null>(null);

  function errorMsg(err: unknown, fallbackKey = 'common.error'): string {
    return err instanceof Error ? err.message : t(fallbackKey);
  }

  const loadExtensions = useCallback(async () => {
    try {
      setLoading(true);
      const response = await apiClient.getExtensions(teamId, {
        page, limit: 20, search: search || undefined, sort,
      });
      setExtensions(response.extensions);
      setTotalPages(response.total_pages ?? 1);
      setTotal(response.total);
      setError('');
    } catch (err) {
      setError(errorMsg(err));
    } finally {
      setLoading(false);
    }
  }, [teamId, page, search, sort, t]);

  useEffect(() => {
    loadExtensions();
  }, [loadExtensions]);

  useEffect(() => {
    setSearch('');
    setSort('updated_at');
    setPage(1);
    setSelectedExt(null);
    setAddToAgentExt(null);
    setExpandedDescriptions(new Set());
    setConfirmAction(null);
    setError('');
  }, [teamId]);

  useEffect(() => { setPage(1); }, [search, sort]);

  async function handleConfirmAction(): Promise<void> {
    if (!confirmAction) return;
    try {
      if (confirmAction.type === 'delete') {
        await apiClient.deleteExtension(confirmAction.id);
      } else {
        await apiClient.uninstallExtension(confirmAction.id);
      }
      loadExtensions();
    } catch (err) {
      setError(errorMsg(err));
    } finally {
      setConfirmAction(null);
    }
  }

  async function handleInstall(extId: string, afterSuccess?: (ext: SharedExtension) => void): Promise<void> {
    setInstallingId(extId);
    try {
      const result = await apiClient.installExtension(extId);
      if (result.success) {
        addToast('success', t('teams.resource.registerSuccess'));
        const ext = extensions.find((item) => item.id === extId);
        if (ext && afterSuccess) afterSuccess(ext);
      } else {
        setError(result.error || t('teams.resource.registerFailed'));
      }
    } catch (err) {
      setError(errorMsg(err));
    } finally {
      setInstallingId(null);
    }
  }

  async function handleReview(extId: string, approved: boolean): Promise<void> {
    try {
      await apiClient.reviewExtension(extId, approved);
      loadExtensions();
    } catch (err) {
      setError(errorMsg(err));
    }
  }

  async function handleAiDescribe(extId: string): Promise<void> {
    if (describingId) return;
    const ext = extensions.find(e => e.id === extId);
    const lang = i18n.language.substring(0, 2);
    if (ext?.aiDescription && ext.aiDescriptionLang === lang) {
      setExpandedDescriptions(prev => {
        const next = new Set(prev);
        if (next.has(extId)) next.delete(extId); else next.add(extId);
        return next;
      });
      return;
    }
    setDescribingId(extId);
    try {
      const result = await apiClient.describeExtension(teamId, extId, lang);
      setExtensions(prev => prev.map(e =>
        e.id === extId ? { ...e, aiDescription: result.description, aiDescriptionLang: result.lang, aiDescribedAt: result.generated_at } : e
      ));
      setExpandedDescriptions(prev => new Set(prev).add(extId));
    } catch (err) {
      setError(errorMsg(err, 'aiInsights.generateError'));
    } finally {
      setDescribingId(null);
    }
  }

  return (
    <>
      <section
        className="ui-section-panel relative mb-4 overflow-hidden px-5 py-5"
        style={{
          background:
            'radial-gradient(circle at top left, hsl(var(--semantic-extension) / 0.14), transparent 42%), linear-gradient(135deg, hsl(var(--background)) 0%, hsl(var(--semantic-extension) / 0.08) 100%)',
        }}
      >
        <div className="pointer-events-none absolute -right-10 top-0 h-28 w-28 rounded-full border border-[hsl(var(--semantic-extension))]/18" />
        <div className="relative flex flex-col gap-5 lg:flex-row lg:items-start lg:justify-between">
          <div className="space-y-3">
            <div className="flex items-center gap-3">
              <div className="flex h-10 w-10 items-center justify-center rounded-full bg-[hsl(var(--semantic-extension))]/10 text-[hsl(var(--semantic-extension))]">
                <Link2 className="h-4 w-4" />
              </div>
              <div className="min-w-0">
                <div className="ui-kicker text-[hsl(var(--semantic-extension))]">
                  {t('teams.resource.mcpWorkspace.launchEyebrow')}
                </div>
                <div className="text-[16px] font-semibold tracking-[-0.02em] text-[hsl(var(--foreground))]">
                  {t('teams.resource.mcpWorkspace.launchTitle')}
                </div>
              </div>
            </div>
            <p className="max-w-3xl text-sm leading-6 text-[hsl(var(--ui-text-secondary))]">
              {t('teams.resource.mcpWorkspace.launchDescription')}
            </p>
            <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs font-medium text-[hsl(var(--ui-text-secondary))]">
              <span className="inline-flex items-center gap-2">
                <span className="h-1.5 w-1.5 rounded-full bg-[hsl(var(--semantic-extension))]" />
                {t('teams.resource.mcpWorkspace.launchFeatureOverview')}
              </span>
              <span className="inline-flex items-center gap-2">
                <span className="h-1.5 w-1.5 rounded-full bg-[hsl(var(--semantic-extension))]" />
                {t('teams.resource.mcpWorkspace.launchFeatureInstall')}
              </span>
              <span className="inline-flex items-center gap-2">
                <span className="h-1.5 w-1.5 rounded-full bg-[hsl(var(--semantic-extension))]" />
                {t('teams.resource.mcpWorkspace.launchFeatureManage')}
              </span>
            </div>
          </div>
          <div className="flex shrink-0 flex-col items-stretch gap-2 sm:flex-row lg:flex-col lg:items-end">
            <Button
              className="min-w-[220px] border-[hsl(var(--semantic-extension))]/24 bg-[hsl(var(--semantic-extension))]/10 text-[hsl(var(--semantic-extension))] shadow-none hover:bg-[hsl(var(--semantic-extension))]/16"
              onClick={() => navigate(`/teams/${teamId}/mcp/workspace`)}
            >
              <Link2 className="mr-2 h-4 w-4" />
              {t('teams.resource.mcpWorkspace.openPage')}
            </Button>
          </div>
        </div>
      </section>
      {loading ? (
        <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>
      ) : error ? (
        <p className="text-center py-8 text-[hsl(var(--destructive))]">{error}</p>
      ) : (
        <>
          <div className="ui-section-panel mb-4 flex flex-wrap items-center gap-3 p-4">
            <div className="relative min-w-[160px] flex-1 sm:min-w-[200px]">
              <Search className="absolute left-2 top-2.5 h-4 w-4 text-[hsl(var(--muted-foreground))]" />
              <Input
                className="w-full pl-8 pr-3 py-2 rounded-md border border-[hsl(var(--border))] bg-[hsl(var(--background))] text-sm"
                placeholder={t('common.search')}
                aria-label={t('common.search')}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <Select value={sort} onValueChange={setSort}>
              <SelectTrigger className="w-full text-sm sm:w-[min(210px,100%)]" aria-label={t('teams.resource.sortLabel', 'Sort by')}>
                <SelectValue placeholder={t('teams.resource.sortUpdated')} />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="updated_at">{t('teams.resource.sortUpdated')}</SelectItem>
                <SelectItem value="name">{t('teams.resource.sortName')}</SelectItem>
                <SelectItem value="created_at">{t('teams.resource.sortCreated')}</SelectItem>
                <SelectItem value="use_count">{t('teams.resource.sortUsage')}</SelectItem>
              </SelectContent>
            </Select>
            {canManage && (
              <Button onClick={() => setCreateDialogOpen(true)}>
                <Plus className="h-4 w-4 mr-2" />
                {t('teams.resource.createExtension')}
              </Button>
            )}
          </div>
          <div className="ui-subtle-panel mb-4 flex items-start gap-2 p-3 text-sm ui-secondary-text">
            <Info className="mt-0.5 h-4 w-4 shrink-0" />
            <p>
              <span className="font-medium text-[hsl(var(--foreground))]">{t('teams.resource.quickTipLabel')}</span>
              {t('teams.resource.quickTip')}
            </p>
          </div>

          {extensions.length === 0 ? (
            <div className="ui-empty-panel">{t('teams.resource.noExtensions')}</div>
          ) : (
          <>
          <div className="space-y-3 md:hidden">
            {extensions.map((ext) => (
              <div
                key={ext.id}
                className="rounded-[22px] border border-[hsl(var(--ui-line-soft))]/80 bg-[hsl(var(--ui-surface-panel))] p-4 shadow-[0_10px_24px_hsl(var(--ui-shadow)/0.05)]"
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate text-[15px] font-semibold text-[hsl(var(--foreground))]">
                      {ext.name}
                    </div>
                    <div className="mt-1 text-xs text-[hsl(var(--ui-text-secondary))]">
                      {ext.authorId}
                    </div>
                  </div>
                  <div
                    className={`shrink-0 rounded-full px-2.5 py-1 text-[11px] font-medium ${
                      ext.securityReviewed
                        ? 'bg-[hsl(var(--status-success-bg))] text-[hsl(var(--status-success-text))]'
                        : 'bg-[hsl(var(--status-warning-bg))] text-[hsl(var(--status-warning-text))]'
                    }`}
                  >
                    {ext.securityReviewed ? t('teams.resource.reviewed') : t('teams.resource.pendingReview')}
                  </div>
                </div>

                <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
                  <div className="rounded-2xl bg-[hsl(var(--ui-surface-panel-muted))] px-3 py-2">
                    <div className="text-[hsl(var(--ui-text-tertiary))]">{t('teams.resource.version')}</div>
                    <div className="mt-1 font-medium text-[hsl(var(--foreground))]">{ext.version}</div>
                  </div>
                  <div className="rounded-2xl bg-[hsl(var(--ui-surface-panel-muted))] px-3 py-2">
                    <div className="text-[hsl(var(--ui-text-tertiary))]">{t('teams.resource.usageCount')}</div>
                    <div className="mt-1 font-medium text-[hsl(var(--foreground))]">{ext.useCount}</div>
                  </div>
                </div>

                <div className="mt-4 flex flex-wrap gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => {
                      setSelectedExt(ext);
                      setDialogMode('view');
                    }}
                  >
                    <Eye className="mr-1.5 h-4 w-4" />
                    {t('common.view')}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => handleAiDescribe(ext.id)}
                    disabled={describingId === ext.id}
                  >
                    <Sparkles className={`mr-1.5 h-4 w-4 ${ext.aiDescription ? 'text-status-warning-text' : ''} ${describingId === ext.id ? 'animate-spin' : ''}`} />
                    {t('aiInsights.describe')}
                  </Button>
                  {canManage ? (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-9 rounded-full px-3"
                      onClick={() => setAddToAgentExt(ext)}
                    >
                      <Bot className="mr-1.5 h-4 w-4" />
                      {t('teams.resource.addToAgent')}
                    </Button>
                  ) : null}
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => handleInstall(ext.id)}
                    disabled={installingId === ext.id}
                  >
                    <Download className="mr-1.5 h-4 w-4" />
                    {t('teams.resource.registerToTeam')}
                  </Button>
                  {canManage ? (
                    <Button
                      variant="outline"
                      size="sm"
                      className="h-9 rounded-full px-3"
                      onClick={() =>
                        handleInstall(ext.id, (installedExt) => {
                          setAutoAssignExt(installedExt);
                        })
                      }
                      disabled={installingId === ext.id}
                    >
                      <Link2 className="mr-1.5 h-4 w-4" />
                      {t('teams.resource.registerAndAddToAgent')}
                    </Button>
                  ) : null}
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-full px-3"
                    onClick={() => setConfirmAction({ type: 'uninstall', id: ext.id })}
                  >
                    <X className="mr-1.5 h-4 w-4" />
                    {t('teams.resource.uninstall')}
                  </Button>
                  {canManage ? (
                    <>
                      {!ext.securityReviewed && (
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-9 rounded-full px-3"
                          onClick={() => handleReview(ext.id, true)}
                        >
                          <ShieldCheck className="mr-1.5 h-4 w-4" />
                          {t('teams.resource.approve')}
                        </Button>
                      )}
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-9 rounded-full px-3"
                        onClick={() => {
                          setSelectedExt(ext);
                          setDialogMode('edit');
                        }}
                      >
                        <Pencil className="mr-1.5 h-4 w-4" />
                        {t('common.edit')}
                      </Button>
                      <Button
                        variant="outline"
                        size="sm"
                        className="h-9 rounded-full px-3 text-[hsl(var(--destructive))] hover:text-[hsl(var(--destructive))]"
                        onClick={() => setConfirmAction({ type: 'delete', id: ext.id })}
                      >
                        <Trash2 className="mr-1.5 h-4 w-4" />
                        {t('common.delete')}
                      </Button>
                    </>
                  ) : null}
                </div>

                {expandedDescriptions.has(ext.id) && ext.aiDescription ? (
                  <div className="mt-4 rounded-2xl bg-[hsl(var(--ui-surface-panel-muted))] px-3 py-3">
                    <div className="text-sm whitespace-pre-wrap text-[hsl(var(--foreground))]">{ext.aiDescription}</div>
                    {ext.aiDescribedAt && (
                      <div className="mt-2 text-xs ui-tertiary-text">
                        {t('aiInsights.generatedAt')}: {formatDateTime(ext.aiDescribedAt)}
                      </div>
                    )}
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
                <TableHead>{t('teams.resource.securityStatus')}</TableHead>
                <TableHead>{t('teams.resource.usageCount')}</TableHead>
                <TableHead className="w-[160px] sm:w-[220px]">{t('common.actions')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {extensions.map((ext) => (
                <Fragment key={ext.id}>
                <TableRow>
                  <TableCell className="font-medium">{ext.name}</TableCell>
                  <TableCell>{ext.authorId}</TableCell>
                  <TableCell>{ext.version}</TableCell>
                  <TableCell>
                    {ext.securityReviewed ? (
                      <span className="text-xs font-medium text-status-success-text">{t('teams.resource.reviewed')}</span>
                    ) : (
                      <span className="text-xs font-medium text-status-warning-text">{t('teams.resource.pendingReview')}</span>
                    )}
                  </TableCell>
                  <TableCell>{ext.useCount}</TableCell>
                  <TableCell>
                    <div className="flex gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setSelectedExt(ext);
                          setDialogMode('view');
                        }}
                        title={t('common.view')}
                      >
                        <Eye className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleAiDescribe(ext.id)}
                        disabled={describingId === ext.id}
                        title={t('aiInsights.describe')}
                      >
                        <Sparkles className={`h-4 w-4 ${ext.aiDescription ? 'text-status-warning-text' : ''} ${describingId === ext.id ? 'animate-spin' : ''}`} />
                      </Button>
                      {canManage && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setAddToAgentExt(ext)}
                          title={t('teams.resource.addToAgent')}
                        >
                          <Bot className="h-4 w-4" />
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleInstall(ext.id)}
                        disabled={installingId === ext.id}
                        title={t('teams.resource.registerToTeam')}
                      >
                        <Download className="h-4 w-4" />
                      </Button>
                      {canManage && (
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            handleInstall(ext.id, (installedExt) => {
                              setAutoAssignExt(installedExt);
                            })
                          }
                          disabled={installingId === ext.id}
                          title={t('teams.resource.registerAndAddToAgent')}
                        >
                          <Link2 className="h-4 w-4" />
                        </Button>
                      )}
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setConfirmAction({ type: 'uninstall', id: ext.id })}
                        title={t('teams.resource.uninstall')}
                      >
                        <X className="h-4 w-4" />
                      </Button>
                      {canManage && (
                        <>
                          {!ext.securityReviewed && (
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => handleReview(ext.id, true)}
                              title={t('teams.resource.approve')}
                            >
                              <ShieldCheck className="h-4 w-4" />
                            </Button>
                          )}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => {
                              setSelectedExt(ext);
                              setDialogMode('edit');
                            }}
                            title={t('common.edit')}
                          >
                            <Pencil className="h-4 w-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setConfirmAction({ type: 'delete', id: ext.id })}
                            title={t('common.delete')}
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
                {expandedDescriptions.has(ext.id) && ext.aiDescription && (
                  <TableRow>
                    <TableCell colSpan={6} className="bg-[hsl(var(--ui-surface-panel-muted))/0.72] p-4">
                      <div className="text-sm whitespace-pre-wrap">{ext.aiDescription}</div>
                      {ext.aiDescribedAt && (
                        <div className="mt-2 text-xs ui-tertiary-text">
                          {t('aiInsights.generatedAt')}: {formatDateTime(ext.aiDescribedAt)}
                        </div>
                      )}
                    </TableCell>
                  </TableRow>
                )}
                </Fragment>
              ))}
            </TableBody>
          </Table>
          </div>

          {totalPages > 1 && (
            <div className="mt-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
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
        </>
      )}

      <ResourceDetailDialog
        open={!!selectedExt}
        onOpenChange={() => setSelectedExt(null)}
        resource={selectedExt}
        resourceType="extension"
        mode={dialogMode}
        onSave={async (data) => {
          if (selectedExt) {
            await apiClient.updateExtension(selectedExt.id, data);
            loadExtensions();
          }
        }}
      />

      <CreateExtensionDialog
        teamId={teamId}
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={loadExtensions}
      />

      <AddExtensionToAgentDialog
        open={!!addToAgentExt}
        onOpenChange={() => setAddToAgentExt(null)}
        extensionId={addToAgentExt?.id ?? ''}
        extensionName={addToAgentExt?.name ?? ''}
        teamId={teamId}
      />
      <AddExtensionToAgentDialog
        open={!!autoAssignExt}
        onOpenChange={() => setAutoAssignExt(null)}
        extensionId={autoAssignExt?.id ?? ''}
        extensionName={autoAssignExt?.name ?? ''}
        teamId={teamId}
      />

      <ConfirmDialog
        open={!!confirmAction}
        onOpenChange={(open) => { if (!open) setConfirmAction(null); }}
        title={t(confirmAction?.type === 'delete' ? 'teams.resource.deleteConfirm' : 'teams.resource.uninstallConfirm')}
        variant="destructive"
        onConfirm={handleConfirmAction}
      />
    </>
  );
}
