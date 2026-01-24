import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { Save, AlertTriangle } from 'lucide-react';
import { SharedSkill, SharedRecipe, SharedExtension } from './types';
import { updateSkill, updateRecipe, updateExtension } from './api';

type ResourceType = 'skill' | 'recipe' | 'extension';
type Resource = SharedSkill | SharedRecipe | SharedExtension;

interface ResourceEditDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  resourceType: ResourceType;
  resource: Resource | null;
  onSuccess: (updated: Resource) => void;
}

interface SkillFormData {
  name: string;
  description: string;
  content: string;
  tags: string;
  visibility: string;
}

interface RecipeFormData {
  name: string;
  description: string;
  contentYaml: string;
  category: string;
  tags: string;
  visibility: string;
}

interface ExtensionFormData {
  name: string;
  description: string;
  config: string;
  tags: string;
  visibility: string;
}

const ResourceEditDialog: React.FC<ResourceEditDialogProps> = ({
  open,
  onOpenChange,
  resourceType,
  resource,
  onSuccess,
}) => {
  const { t } = useTranslation('team');
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Form states
  const [skillForm, setSkillForm] = useState<SkillFormData>({
    name: '',
    description: '',
    content: '',
    tags: '',
    visibility: 'team',
  });

  const [recipeForm, setRecipeForm] = useState<RecipeFormData>({
    name: '',
    description: '',
    contentYaml: '',
    category: '',
    tags: '',
    visibility: 'team',
  });

  const [extensionForm, setExtensionForm] = useState<ExtensionFormData>({
    name: '',
    description: '',
    config: '{}',
    tags: '',
    visibility: 'team',
  });

  // Initialize form when resource changes
  useEffect(() => {
    if (!resource) return;

    switch (resourceType) {
      case 'skill': {
        const skill = resource as SharedSkill;
        setSkillForm({
          name: skill.name,
          description: skill.description || '',
          content: skill.content || '',
          tags: skill.tags.join(', '),
          visibility: skill.visibility || 'team',
        });
        break;
      }
      case 'recipe': {
        const recipe = resource as SharedRecipe;
        setRecipeForm({
          name: recipe.name,
          description: recipe.description || '',
          contentYaml: recipe.contentYaml,
          category: recipe.category || '',
          tags: recipe.tags.join(', '),
          visibility: recipe.visibility || 'team',
        });
        break;
      }
      case 'extension': {
        const extension = resource as SharedExtension;
        setExtensionForm({
          name: extension.name,
          description: extension.description || '',
          config: JSON.stringify(extension.config, null, 2),
          tags: extension.tags.join(', '),
          visibility: extension.visibility || 'team',
        });
        break;
      }
    }
  }, [resource, resourceType]);

  const getTitle = () => {
    switch (resourceType) {
      case 'skill':
        return t('manage.editSkill');
      case 'recipe':
        return t('manage.editRecipe');
      case 'extension':
        return t('manage.editExtension');
    }
  };

  const parseTags = (tagsStr: string): string[] => {
    return tagsStr
      .split(',')
      .map((tag) => tag.trim())
      .filter((tag) => tag.length > 0);
  };

  const handleSave = async () => {
    if (!resource) return;

    setIsSaving(true);
    setError(null);

    try {
      let updated: Resource;

      switch (resourceType) {
        case 'skill': {
          updated = await updateSkill(resource.id, {
            name: skillForm.name,
            description: skillForm.description || undefined,
            content: skillForm.content,
            tags: parseTags(skillForm.tags),
            visibility: skillForm.visibility,
          });
          break;
        }
        case 'recipe': {
          updated = await updateRecipe(resource.id, {
            name: recipeForm.name,
            description: recipeForm.description || undefined,
            contentYaml: recipeForm.contentYaml,
            category: recipeForm.category || undefined,
            tags: parseTags(recipeForm.tags),
            visibility: recipeForm.visibility,
          });
          break;
        }
        case 'extension': {
          let config: Record<string, unknown>;
          try {
            config = JSON.parse(extensionForm.config);
          } catch {
            setError('Invalid JSON configuration');
            setIsSaving(false);
            return;
          }
          updated = await updateExtension(resource.id, {
            name: extensionForm.name,
            description: extensionForm.description || undefined,
            config,
            tags: parseTags(extensionForm.tags),
            visibility: extensionForm.visibility,
          });
          break;
        }
      }

      onSuccess(updated!);
      onOpenChange(false);
    } catch (err) {
      console.error('Failed to save:', err);
      setError(t('manage.saveError'));
    } finally {
      setIsSaving(false);
    }
  };

  const renderFormField = (
    label: string,
    children: React.ReactNode,
    required = false
  ) => (
    <div className="space-y-1.5">
      <label className="text-sm font-medium text-text-default">
        {label}
        {required && <span className="text-red-500 ml-1">*</span>}
      </label>
      {children}
    </div>
  );

  const renderVisibilitySelect = (
    value: string,
    onChange: (value: string) => void
  ) => (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="w-full px-3 py-2 text-sm border border-border-default rounded-md bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500"
    >
      <option value="team">{t('edit.visibilityTeam')}</option>
      <option value="public">{t('edit.visibilityPublic')}</option>
    </select>
  );

  const renderSkillForm = () => (
    <div className="space-y-4">
      {renderFormField(
        t('detail.name'),
        <Input
          value={skillForm.name}
          onChange={(e) => setSkillForm({ ...skillForm, name: e.target.value })}
          placeholder={t('edit.namePlaceholder')}
        />,
        true
      )}
      {renderFormField(
        t('detail.description'),
        <Textarea
          value={skillForm.description}
          onChange={(e) => setSkillForm({ ...skillForm, description: e.target.value })}
          placeholder={t('edit.descriptionPlaceholder')}
          rows={2}
        />
      )}
      {renderFormField(
        t('detail.content'),
        <Textarea
          value={skillForm.content}
          onChange={(e) => setSkillForm({ ...skillForm, content: e.target.value })}
          placeholder={t('edit.contentPlaceholder')}
          rows={8}
          className="font-mono text-sm"
        />,
        true
      )}
      {renderFormField(
        t('detail.tags'),
        <Input
          value={skillForm.tags}
          onChange={(e) => setSkillForm({ ...skillForm, tags: e.target.value })}
          placeholder={t('edit.tagsPlaceholder')}
        />
      )}
      {renderFormField(
        t('detail.visibility'),
        renderVisibilitySelect(skillForm.visibility, (v) =>
          setSkillForm({ ...skillForm, visibility: v })
        )
      )}
    </div>
  );

  const renderRecipeForm = () => (
    <div className="space-y-4">
      {renderFormField(
        t('detail.name'),
        <Input
          value={recipeForm.name}
          onChange={(e) => setRecipeForm({ ...recipeForm, name: e.target.value })}
          placeholder={t('edit.namePlaceholder')}
        />,
        true
      )}
      {renderFormField(
        t('detail.description'),
        <Textarea
          value={recipeForm.description}
          onChange={(e) => setRecipeForm({ ...recipeForm, description: e.target.value })}
          placeholder={t('edit.descriptionPlaceholder')}
          rows={2}
        />
      )}
      {renderFormField(
        t('detail.content') + ' (YAML)',
        <Textarea
          value={recipeForm.contentYaml}
          onChange={(e) => setRecipeForm({ ...recipeForm, contentYaml: e.target.value })}
          placeholder={t('edit.contentPlaceholder')}
          rows={8}
          className="font-mono text-sm"
        />,
        true
      )}
      {renderFormField(
        t('detail.category'),
        <Input
          value={recipeForm.category}
          onChange={(e) => setRecipeForm({ ...recipeForm, category: e.target.value })}
          placeholder={t('edit.categoryPlaceholder')}
        />
      )}
      {renderFormField(
        t('detail.tags'),
        <Input
          value={recipeForm.tags}
          onChange={(e) => setRecipeForm({ ...recipeForm, tags: e.target.value })}
          placeholder={t('edit.tagsPlaceholder')}
        />
      )}
      {renderFormField(
        t('detail.visibility'),
        renderVisibilitySelect(recipeForm.visibility, (v) =>
          setRecipeForm({ ...recipeForm, visibility: v })
        )
      )}
    </div>
  );

  const renderExtensionForm = () => (
    <div className="space-y-4">
      {renderFormField(
        t('detail.name'),
        <Input
          value={extensionForm.name}
          onChange={(e) => setExtensionForm({ ...extensionForm, name: e.target.value })}
          placeholder={t('edit.namePlaceholder')}
        />,
        true
      )}
      {renderFormField(
        t('detail.description'),
        <Textarea
          value={extensionForm.description}
          onChange={(e) => setExtensionForm({ ...extensionForm, description: e.target.value })}
          placeholder={t('edit.descriptionPlaceholder')}
          rows={2}
        />
      )}
      {renderFormField(
        t('detail.config') + ' (JSON)',
        <Textarea
          value={extensionForm.config}
          onChange={(e) => setExtensionForm({ ...extensionForm, config: e.target.value })}
          placeholder="{}"
          rows={8}
          className="font-mono text-sm"
        />,
        true
      )}
      {renderFormField(
        t('detail.tags'),
        <Input
          value={extensionForm.tags}
          onChange={(e) => setExtensionForm({ ...extensionForm, tags: e.target.value })}
          placeholder={t('edit.tagsPlaceholder')}
        />
      )}
      {renderFormField(
        t('detail.visibility'),
        renderVisibilitySelect(extensionForm.visibility, (v) =>
          setExtensionForm({ ...extensionForm, visibility: v })
        )
      )}
    </div>
  );

  const renderForm = () => {
    switch (resourceType) {
      case 'skill':
        return renderSkillForm();
      case 'recipe':
        return renderRecipeForm();
      case 'extension':
        return renderExtensionForm();
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl max-h-[80vh] overflow-hidden flex flex-col">
        <DialogHeader>
          <DialogTitle>{getTitle()}</DialogTitle>
        </DialogHeader>
        <div className="flex-1 overflow-y-auto pr-2 py-4">
          {renderForm()}
          {error && (
            <div className="mt-4 p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-md flex items-center gap-2">
              <AlertTriangle size={16} className="text-red-500" />
              <span className="text-sm text-red-600 dark:text-red-400">{error}</span>
            </div>
          )}
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={isSaving}
          >
            {t('cancel')}
          </Button>
          <Button onClick={handleSave} disabled={isSaving}>
            {isSaving ? (
              <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full mr-2" />
            ) : (
              <Save size={14} className="mr-2" />
            )}
            {isSaving ? t('manage.saving') : t('manage.save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

export default ResourceEditDialog;
