import { useEffect, useState } from 'react';
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { Loader2 } from 'lucide-react';
import { apiClient } from '../../api/client';

interface Props {
  teamId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}

const DEFAULT_STDIO_CONFIG = '{\n  "uri_or_cmd": "",\n  "args": [],\n  "envs": {}\n}';
const DEFAULT_SSE_CONFIG = '{\n  "uri_or_cmd": "http://127.0.0.1:3000/sse",\n  "args": [],\n  "envs": {}\n}';
const DEFAULT_BUILTIN_CONFIG = '{\n  "uri_or_cmd": "",\n  "args": [],\n  "envs": {}\n}';

function getDefaultConfigByType(extensionType: string): string {
  if (extensionType === 'sse') return DEFAULT_SSE_CONFIG;
  if (extensionType === 'builtin') return DEFAULT_BUILTIN_CONFIG;
  return DEFAULT_STDIO_CONFIG;
}

export function CreateExtensionDialog({ teamId, open, onOpenChange, onCreated }: Props) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [extensionType, setExtensionType] = useState('stdio');
  const [configJson, setConfigJson] = useState(getDefaultConfigByType('stdio'));
  const [tags, setTags] = useState('');

  useEffect(() => {
    const defaults = [DEFAULT_STDIO_CONFIG, DEFAULT_SSE_CONFIG, DEFAULT_BUILTIN_CONFIG].map((v) =>
      v.trim()
    );
    setConfigJson((prev) => {
      const current = prev.trim();
      if (current === '' || defaults.includes(current)) {
        return getDefaultConfigByType(extensionType);
      }
      return prev;
    });
  }, [extensionType]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim() || !configJson.trim()) return;

    let config;
    try {
      config = JSON.parse(configJson);
    } catch {
      setError(t('teams.resource.invalidJson'));
      return;
    }

    setLoading(true);
    setError('');
    try {
      await apiClient.createExtension({
        teamId,
        name: name.trim(),
        extensionType,
        config,
        description: description.trim() || undefined,
        tags: tags ? tags.split(',').map(t => t.trim()).filter(Boolean) : undefined,
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
    setExtensionType('stdio');
    setConfigJson(getDefaultConfigByType('stdio'));
    setTags('');
    setError('');
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[600px]">
        <DialogHeader>
          <DialogTitle>{t('teams.resource.createExtension')}</DialogTitle>
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
                placeholder={t('teams.resource.extensionNamePlaceholder')}
                required
              />
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
              <Label>{t('teams.resource.extensionType')}</Label>
              <Select value={extensionType} onValueChange={setExtensionType}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="stdio">Stdio (MCP)</SelectItem>
                  <SelectItem value="sse">SSE</SelectItem>
                  <SelectItem value="builtin">Builtin</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="config">{t('teams.resource.config')} *</Label>
              <Textarea
                id="config"
                value={configJson}
                onChange={(e) => setConfigJson(e.target.value)}
                placeholder={t('teams.resource.extensionConfigPlaceholder')}
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
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={loading || !name.trim() || !configJson.trim()}>
              {loading && <Loader2 className="w-4 h-4 animate-spin mr-1.5" />}
              {loading ? t('common.creating') : t('common.create')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
