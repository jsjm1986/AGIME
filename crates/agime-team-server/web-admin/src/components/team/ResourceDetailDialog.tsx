import { useState, useEffect } from 'react';
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
import type { SharedSkill, SharedRecipe, SharedExtension } from '../../api/types';

type Resource = SharedSkill | SharedRecipe | SharedExtension | null;

interface ResourceDetailDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  resource: Resource;
  resourceType: 'skill' | 'recipe' | 'extension';
  mode: 'view' | 'edit';
  onSave: (data: { name?: string; description?: string; content?: string; config?: string }) => Promise<void>;
}

type TabKey = 'content' | 'skillMd' | 'files';

export function ResourceDetailDialog({
  open,
  onOpenChange,
  resource,
  resourceType,
  mode,
  onSave,
}: ResourceDetailDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [content, setContent] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [activeTab, setActiveTab] = useState<TabKey>('content');

  const isSkill = resourceType === 'skill';
  const skill = isSkill ? (resource as SharedSkill | null) : null;

  useEffect(() => {
    if (resource) {
      setName(resource.name);
      setDescription(resource.description || '');
      if ('content' in resource && resource.content) {
        setContent(resource.content);
      } else if ('contentYaml' in resource) {
        setContent(resource.contentYaml);
      } else if ('config' in resource) {
        setContent(JSON.stringify(resource.config, null, 2));
      }
      setActiveTab('content');
    }
  }, [resource]);

  const handleSave = async () => {
    setSaving(true);
    setError('');
    try {
      const data: { name?: string; description?: string; content?: string; config?: string } = {
        name,
        description: description || undefined,
      };
      if (resourceType === 'extension') {
        data.config = content;
      } else {
        data.content = content;
      }
      await onSave(data);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  if (!resource) return null;

  const isEditing = mode === 'edit';
  const contentLabel = resourceType === 'extension' ? t('teams.resource.config') : t('teams.resource.content');

  const tabs: { key: TabKey; label: string }[] = [
    { key: 'content', label: contentLabel },
  ];
  if (isSkill) {
    tabs.push({ key: 'skillMd', label: t('teams.resource.skillMd') });
    tabs.push({ key: 'files', label: t('teams.resource.filesTab') });
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>
            {isEditing ? t('teams.resource.edit') : t('teams.resource.view')} - {resource.name}
          </DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.resource.name')}</label>
            {isEditing ? (
              <Input value={name} onChange={(e) => setName(e.target.value)} />
            ) : (
              <p className="text-sm">{resource.name}</p>
            )}
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.resource.description')}</label>
            {isEditing ? (
              <Textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={2} />
            ) : (
              <p className="text-sm text-[hsl(var(--muted-foreground))]">
                {resource.description || t('teams.resource.noDescription')}
              </p>
            )}
          </div>

          <div className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <span className="text-[hsl(var(--muted-foreground))]">{t('teams.resource.author')}:</span>{' '}
              {resource.authorId}
            </div>
            <div>
              <span className="text-[hsl(var(--muted-foreground))]">{t('teams.resource.version')}:</span>{' '}
              {resource.version}
            </div>
          </div>

          {/* Tab bar */}
          {tabs.length > 1 && (
            <div className="flex gap-1 border-b border-[hsl(var(--border))]">
              {tabs.map((tab) => (
                <button
                  key={tab.key}
                  type="button"
                  className={`px-3 py-1.5 text-sm transition-colors ${
                    activeTab === tab.key
                      ? 'border-b-2 border-[hsl(var(--primary))] text-[hsl(var(--foreground))] font-medium'
                      : 'text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]'
                  }`}
                  onClick={() => setActiveTab(tab.key)}
                >
                  {tab.label}
                </button>
              ))}
            </div>
          )}

          {/* Content tab */}
          {activeTab === 'content' && (
            <div className="space-y-2">
              {tabs.length <= 1 && (
                <label className="text-sm font-medium">{contentLabel}</label>
              )}
              {isEditing ? (
                <Textarea
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                  rows={10}
                  className="font-mono text-sm"
                />
              ) : (
                <pre className="p-3 bg-[hsl(var(--muted))] rounded-md text-sm overflow-x-auto max-h-64">
                  {content}
                </pre>
              )}
            </div>
          )}

          {/* SKILL.md tab */}
          {activeTab === 'skillMd' && skill && (
            <div className="space-y-2">
              {skill.skillMd ? (
                <pre className="p-3 bg-[hsl(var(--muted))] rounded-md text-xs font-mono overflow-x-auto max-h-80 overflow-y-auto whitespace-pre-wrap">
                  {skill.skillMd}
                </pre>
              ) : (
                <p className="text-sm text-[hsl(var(--muted-foreground))] italic">
                  {t('teams.resource.noSkillMd')}
                </p>
              )}
            </div>
          )}

          {/* Files tab */}
          {activeTab === 'files' && skill && (
            <div className="space-y-2">
              {skill.files && skill.files.length > 0 ? (
                <div className="border border-[hsl(var(--border))] rounded-md divide-y divide-[hsl(var(--border))]">
                  {skill.files.map((file) => (
                    <div key={file.path} className="px-3 py-2 text-sm flex items-center justify-between">
                      <span className="font-mono text-xs">{file.path}</span>
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-sm text-[hsl(var(--muted-foreground))] italic">
                  {t('teams.resource.noFiles')}
                </p>
              )}
            </div>
          )}

          {error && <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {isEditing ? t('common.cancel') : t('common.close')}
          </Button>
          {isEditing && (
            <Button onClick={handleSave} disabled={saving}>
              {saving ? t('common.saving') : t('common.save')}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
