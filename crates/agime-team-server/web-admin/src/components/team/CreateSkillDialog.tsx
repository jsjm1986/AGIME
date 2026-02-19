import { useState, useMemo } from 'react';
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
import { Label } from '../ui/label';
import { Textarea } from '../ui/textarea';
import { Loader2 } from 'lucide-react';
import { apiClient } from '../../api/client';

const KEBAB_CASE_RE = /^[a-z0-9]+(-[a-z0-9]+)*$/;

function isValidKebabCase(name: string): boolean {
  return name.length > 0 && name.length <= 64 && KEBAB_CASE_RE.test(name);
}

function toKebabCase(input: string): string {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-')
    .slice(0, 64);
}

interface Props {
  teamId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}

export function CreateSkillDialog({ teamId, open, onOpenChange, onCreated }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [content, setContent] = useState('');
  const [tags, setTags] = useState('');

  const nameError = useMemo(() => {
    if (!name) return '';
    if (!isValidKebabCase(name)) return t('teams.resource.kebabCaseError');
    return '';
  }, [name, t]);

  const suggestedName = useMemo(() => {
    if (!name || isValidKebabCase(name)) return '';
    const converted = toKebabCase(name);
    return converted && converted !== name ? converted : '';
  }, [name]);

  // Best-effort preview; the backend generates the canonical SKILL.md via PackageService
  const skillMdPreview = useMemo(() => {
    if (!name.trim() || !content.trim()) return '';
    return `---\nname: ${name.trim()}\ndescription: ${description.trim() || ''}\nmetadata:\n  version: '1.0.0'\n---\n\n${content.trim()}`;
  }, [name, description, content]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !content.trim() || nameError) return;

    setLoading(true);
    setError('');
    try {
      await apiClient.createSkill({
        teamId,
        name: name.trim(),
        content: content.trim(),
        description: description.trim() || undefined,
        tags: tags ? tags.split(',').map(tag => tag.trim()).filter(Boolean) : undefined,
      });
      onCreated();
      onOpenChange(false);
      resetForm();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const resetForm = () => {
    setName('');
    setDescription('');
    setContent('');
    setTags('');
    setError('');
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[700px] max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{t('teams.resource.createSkill')}</DialogTitle>
        </DialogHeader>
        <form onSubmit={handleSubmit}>
          <div className="space-y-4 py-4">
            {error && (
              <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>
            )}
            <div className="space-y-2">
              <Label htmlFor="name">{t('teams.resource.name')} *</Label>
              <Input
                id="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="my-skill-name"
                required
                className={nameError ? 'border-[hsl(var(--destructive))]' : ''}
              />
              {nameError && (
                <p className="text-xs text-[hsl(var(--destructive))]">{nameError}</p>
              )}
              {suggestedName && (
                <p className="text-xs text-[hsl(var(--muted-foreground))]">
                  {t('teams.resource.suggestion')}{' '}
                  <button
                    type="button"
                    className="underline text-[hsl(var(--primary))] cursor-pointer"
                    onClick={() => setName(suggestedName)}
                  >
                    {suggestedName}
                  </button>
                </p>
              )}
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">{t('teams.resource.description')}</Label>
              <Input
                id="description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder={t('teams.resource.descriptionPlaceholder')}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="content">{t('teams.resource.content')} *</Label>
              <Textarea
                id="content"
                value={content}
                onChange={(e) => setContent(e.target.value)}
                placeholder={t('teams.resource.skillContentPlaceholder')}
                rows={8}
                className="font-mono text-sm"
                required
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="tags">{t('teams.resource.tags')}</Label>
              <Input
                id="tags"
                value={tags}
                onChange={(e) => setTags(e.target.value)}
                placeholder={t('teams.resource.tagsPlaceholder')}
              />
            </div>
            {skillMdPreview && (
              <div className="space-y-2">
                <Label>{t('teams.resource.skillMdPreview')}</Label>
                <pre className="p-3 bg-[hsl(var(--muted))] rounded-md text-xs font-mono overflow-x-auto max-h-48 overflow-y-auto">
                  {skillMdPreview}
                </pre>
              </div>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={loading || !name.trim() || !content.trim() || !!nameError}>
              {loading && <Loader2 className="w-4 h-4 animate-spin mr-1.5" />}
              {loading ? t('common.creating') : t('common.create')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
