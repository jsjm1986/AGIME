import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useToast } from '../../contexts/ToastContext';
import { Eye, Pencil, Trash2, Plus, Download, X, Search, ChevronLeft, ChevronRight } from 'lucide-react';
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
import { CreateRecipeDialog } from './CreateRecipeDialog';
import { apiClient } from '../../api/client';
import type { SharedRecipe } from '../../api/types';

type ConfirmAction = { type: 'delete' | 'uninstall'; id: string } | null;

interface RecipesTabProps {
  teamId: string;
  canManage: boolean;
}

export function RecipesTab({ teamId, canManage }: RecipesTabProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [recipes, setRecipes] = useState<SharedRecipe[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedRecipe, setSelectedRecipe] = useState<SharedRecipe | null>(null);
  const [dialogMode, setDialogMode] = useState<'view' | 'edit'>('view');
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState('updated_at');
  const [page, setPage] = useState(1);
  const [totalPages, setTotalPages] = useState(1);
  const [total, setTotal] = useState(0);
  const [confirmAction, setConfirmAction] = useState<ConfirmAction>(null);

  const loadRecipes = useCallback(async () => {
    try {
      setLoading(true);
      const response = await apiClient.getRecipes(teamId, {
        page, limit: 20, search: search || undefined, sort,
      });
      setRecipes(response.recipes);
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
    loadRecipes();
  }, [loadRecipes]);

  useEffect(() => {
    setSearch('');
    setSort('updated_at');
    setPage(1);
    setSelectedRecipe(null);
    setError('');
  }, [teamId]);

  useEffect(() => { setPage(1); }, [search, sort]);

  function errorMsg(err: unknown): string {
    return err instanceof Error ? err.message : t('common.error');
  }

  const handleConfirmAction = async () => {
    if (!confirmAction) return;
    try {
      if (confirmAction.type === 'delete') {
        await apiClient.deleteRecipe(confirmAction.id);
      } else {
        await apiClient.uninstallRecipe(confirmAction.id);
      }
      loadRecipes();
    } catch (err) {
      setError(errorMsg(err));
    } finally {
      setConfirmAction(null);
    }
  };

  const handleInstall = async (recipeId: string) => {
    setInstallingId(recipeId);
    try {
      const result = await apiClient.installRecipe(recipeId);
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
          <Button onClick={() => setCreateDialogOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            {t('teams.resource.createRecipe')}
          </Button>
        )}
      </div>

      {recipes.length === 0 ? (
        <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.resource.noRecipes')}</p>
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
          {recipes.map((recipe) => (
            <TableRow key={recipe.id}>
              <TableCell className="font-medium">{recipe.name}</TableCell>
              <TableCell>{recipe.authorId}</TableCell>
              <TableCell>{recipe.version}</TableCell>
              <TableCell>{recipe.useCount}</TableCell>
              <TableCell>
                <div className="flex gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setSelectedRecipe(recipe);
                      setDialogMode('view');
                    }}
                    title={t('common.view')}
                  >
                    <Eye className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleInstall(recipe.id)}
                    disabled={installingId === recipe.id}
                    title={t('teams.resource.install')}
                  >
                    <Download className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setConfirmAction({ type: 'uninstall', id: recipe.id })}
                    title={t('teams.resource.uninstall')}
                  >
                    <X className="h-4 w-4" />
                  </Button>
                  {canManage && (
                    <>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setSelectedRecipe(recipe);
                          setDialogMode('edit');
                        }}
                        title={t('common.edit')}
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => setConfirmAction({ type: 'delete', id: recipe.id })}
                        title={t('common.delete')}
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </>
                  )}
                </div>
              </TableCell>
            </TableRow>
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
        open={!!selectedRecipe}
        onOpenChange={() => setSelectedRecipe(null)}
        resource={selectedRecipe}
        resourceType="recipe"
        mode={dialogMode}
        onSave={async (data) => {
          if (selectedRecipe) {
            await apiClient.updateRecipe(selectedRecipe.id, data);
            loadRecipes();
          }
        }}
      />

      <CreateRecipeDialog
        teamId={teamId}
        open={createDialogOpen}
        onOpenChange={setCreateDialogOpen}
        onCreated={loadRecipes}
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
