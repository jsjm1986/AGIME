import { useState, useMemo, useRef, useCallback } from 'react';
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
import { Loader2, Upload, X } from 'lucide-react';
import { apiClient } from '../../api/client';
import type { SkillFile } from '../../api/types';

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
  const [mode, setMode] = useState<'inline' | 'package'>('inline');
  const [files, setFiles] = useState<SkillFile[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);

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

  const readFilesAsSkillFiles = useCallback(async (fileList: FileList) => {
    const results: SkillFile[] = [];
    for (const file of Array.from(fileList)) {
      const text = await file.text();
      results.push({ path: file.name, content: text });
    }
    setFiles(prev => [...prev, ...results]);
  }, []);

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
      const base = {
        teamId,
        name: name.trim(),
        description: description.trim() || undefined,
        tags: tags ? tags.split(',').map(tag => tag.trim()).filter(Boolean) : undefined,
      };
      if (mode === 'package') {
        await apiClient.createSkill({
          ...base,
          content: content.trim(),
          skillMd: skillMdPreview,
          files,
        });
      } else {
        await apiClient.createSkill({
          ...base,
          content: content.trim(),
        });
      }
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
    setFiles([]);
    setMode('inline');
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
            <div className="flex gap-2">
              <Button
                type="button"
                size="sm"
                variant={mode === 'inline' ? 'default' : 'outline'}
                onClick={() => setMode('inline')}
              >
                {t('teams.resource.storageInline')}
              </Button>
              <Button
                type="button"
                size="sm"
                variant={mode === 'package' ? 'default' : 'outline'}
                onClick={() => setMode('package')}
              >
                {t('teams.resource.storagePackage')}
              </Button>
            </div>
            {mode === 'package' && (
              <p className="text-xs text-[hsl(var(--muted-foreground))]">{t('teams.resource.packageModeHint')}</p>
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
            {mode === 'package' && (
              <div className="space-y-2">
                <Label>{t('teams.resource.fileList')}</Label>
                <input
                  ref={fileInputRef}
                  type="file"
                  multiple
                  className="hidden"
                  onChange={(e) => {
                    if (e.target.files?.length) {
                      readFilesAsSkillFiles(e.target.files);
                      e.target.value = '';
                    }
                  }}
                />
                <div
                  className="border-2 border-dashed rounded-md p-4 text-center cursor-pointer hover:border-[hsl(var(--primary))] transition-colors"
                  onClick={() => fileInputRef.current?.click()}
                  onDragOver={(e) => e.preventDefault()}
                  onDrop={(e) => {
                    e.preventDefault();
                    if (e.dataTransfer.files.length) readFilesAsSkillFiles(e.dataTransfer.files);
                  }}
                >
                  <Upload className="w-5 h-5 mx-auto mb-1 text-[hsl(var(--muted-foreground))]" />
                  <p className="text-xs text-[hsl(var(--muted-foreground))]">{t('teams.resource.dropFilesHint')}</p>
                </div>
                {files.length > 0 && (
                  <div className="space-y-1">
                    {files.map((f, i) => (
                      <div key={i} className="flex items-center justify-between text-sm px-2 py-1 bg-[hsl(var(--muted))] rounded">
                        <span className="truncate font-mono text-xs">{f.path}</span>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="h-6 w-6 p-0"
                          onClick={() => setFiles(prev => prev.filter((_, idx) => idx !== i))}
                        >
                          <X className="w-3 h-3" />
                        </Button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
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
