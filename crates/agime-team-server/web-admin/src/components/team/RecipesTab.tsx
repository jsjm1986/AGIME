import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Eye, Pencil, Trash2 } from 'lucide-react';
import { Button } from '../ui/button';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../ui/table';
import { ResourceDetailDialog } from './ResourceDetailDialog';
import { apiClient } from '../../api/client';
import type { SharedRecipe } from '../../api/types';

interface RecipesTabProps {
  teamId: string;
  canManage: boolean;
}

export function RecipesTab({ teamId, canManage }: RecipesTabProps) {
  const { t } = useTranslation();
  const [recipes, setRecipes] = useState<SharedRecipe[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedRecipe, setSelectedRecipe] = useState<SharedRecipe | null>(null);
  const [dialogMode, setDialogMode] = useState<'view' | 'edit'>('view');

  const loadRecipes = async () => {
    try {
      setLoading(true);
      const response = await apiClient.getRecipes(teamId);
      setRecipes(response.recipes);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadRecipes();
  }, [teamId]);

  const handleDelete = async (recipeId: string) => {
    if (!confirm(t('teams.resource.deleteConfirm'))) return;
    try {
      await apiClient.deleteRecipe(recipeId);
      loadRecipes();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    }
  };

  if (loading) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>;
  }

  if (error) {
    return <p className="text-center py-8 text-[hsl(var(--destructive))]">{error}</p>;
  }

  if (recipes.length === 0) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.resource.noRecipes')}</p>;
  }

  return (
    <>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t('teams.resource.name')}</TableHead>
            <TableHead>{t('teams.resource.author')}</TableHead>
            <TableHead>{t('teams.resource.version')}</TableHead>
            <TableHead>{t('teams.resource.usageCount')}</TableHead>
            <TableHead className="w-[120px]">{t('common.actions')}</TableHead>
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
                  >
                    <Eye className="h-4 w-4" />
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
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDelete(recipe.id)}
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
    </>
  );
}
