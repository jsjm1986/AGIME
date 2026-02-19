import { Fragment, useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useToast } from '../../contexts/ToastContext';
import { Eye, Pencil, Trash2, Plus, Download, X, Search, ChevronLeft, ChevronRight, ShieldCheck, Bot, Sparkles } from 'lucide-react';
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
import { CreateExtensionDialog } from './CreateExtensionDialog';
import { AddExtensionToAgentDialog } from './AddExtensionToAgentDialog';
import { apiClient } from '../../api/client';
import type { SharedExtension } from '../../api/types';

type ConfirmAction = { type: 'delete' | 'uninstall'; id: string } | null;

interface ExtensionsTabProps {
  teamId: string;
  canManage: boolean;
}

export function ExtensionsTab({ teamId, canManage }: ExtensionsTabProps) {
  const { t, i18n } = useTranslation();
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

  async function handleInstall(extId: string): Promise<void> {
    setInstallingId(extId);
    try {
      const result = await apiClient.installExtension(extId);
      if (result.success) {
        addToast('success', t('teams.resource.installSuccess'));
      } else {
        setError(result.error || t('teams.resource.installFailed'));
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
          <Button onClick={() => setCreateDialogOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            {t('teams.resource.createExtension')}
          </Button>
        )}
      </div>

      {extensions.length === 0 ? (
        <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.resource.noExtensions')}</p>
      ) : (
      <>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t('teams.resource.name')}</TableHead>
            <TableHead>{t('teams.resource.author')}</TableHead>
            <TableHead>{t('teams.resource.version')}</TableHead>
            <TableHead>{t('teams.resource.securityStatus')}</TableHead>
            <TableHead>{t('teams.resource.usageCount')}</TableHead>
            <TableHead className="w-[220px]">{t('common.actions')}</TableHead>
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
                  <span className="text-green-600 text-xs font-medium">{t('teams.resource.reviewed')}</span>
                ) : (
                  <span className="text-yellow-600 text-xs font-medium">{t('teams.resource.pendingReview')}</span>
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
                    <Sparkles className={`h-4 w-4 ${ext.aiDescription ? 'text-amber-500' : ''} ${describingId === ext.id ? 'animate-spin' : ''}`} />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setAddToAgentExt(ext)}
                    title={t('teams.resource.addToAgent')}
                  >
                    <Bot className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleInstall(ext.id)}
                    disabled={installingId === ext.id}
                    title={t('teams.resource.install')}
                  >
                    <Download className="h-4 w-4" />
                  </Button>
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
                <TableCell colSpan={6} className="bg-[hsl(var(--muted))] p-4">
                  <div className="text-sm whitespace-pre-wrap">{ext.aiDescription}</div>
                  {ext.aiDescribedAt && (
                    <div className="text-xs text-[hsl(var(--muted-foreground))] mt-2">
                      {t('aiInsights.generatedAt')}: {new Date(ext.aiDescribedAt).toLocaleString()}
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
