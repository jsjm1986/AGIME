import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Textarea } from '../ui/textarea';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { apiClient } from '../../api/client';
import type { TeamWithStats } from '../../api/types';

interface SettingsTabProps {
  team: TeamWithStats;
  onUpdate: () => void;
  onDelete: () => void;
}

export function SettingsTab({ team, onUpdate, onDelete }: SettingsTabProps) {
  const { t } = useTranslation();
  const [name, setName] = useState(team.name);
  const [description, setDescription] = useState(team.description || '');
  const [repoUrl, setRepoUrl] = useState(team.repositoryUrl || '');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const handleSave = async () => {
    if (!name.trim()) return;
    setSaving(true);
    setError('');
    try {
      await apiClient.updateTeam(team.id, {
        name: name.trim(),
        description: description.trim() || undefined,
        repositoryUrl: repoUrl.trim() || undefined,
      });
      onUpdate();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    setDeleting(true);
    try {
      await apiClient.deleteTeam(team.id);
      onDelete();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
      setDeleting(false);
    }
  };

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle>{t('teams.settings.teamInfo')}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.teamName')} *</label>
            <Input value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.description')}</label>
            <Textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
            />
          </div>
          <div className="space-y-2">
            <label className="text-sm font-medium">{t('teams.repoUrl')}</label>
            <Input value={repoUrl} onChange={(e) => setRepoUrl(e.target.value)} />
          </div>
          {error && <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>}
          <Button onClick={handleSave} disabled={saving || !name.trim()}>
            {saving ? t('common.saving') : t('common.save')}
          </Button>
        </CardContent>
      </Card>

      <Card className="border-[hsl(var(--destructive))]">
        <CardHeader>
          <CardTitle className="text-[hsl(var(--destructive))]">
            {t('teams.settings.dangerZone')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-[hsl(var(--muted-foreground))] mb-4">
            {t('teams.settings.deleteWarning')}
          </p>
          <Button variant="destructive" onClick={() => setDeleteDialogOpen(true)}>
            {t('teams.settings.deleteTeam')}
          </Button>
        </CardContent>
      </Card>

      <Dialog open={deleteDialogOpen} onOpenChange={setDeleteDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('teams.settings.deleteConfirmTitle')}</DialogTitle>
          </DialogHeader>
          <p className="py-4">{t('teams.settings.deleteConfirmMessage', { name: team.name })}</p>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteDialogOpen(false)}>
              {t('common.cancel')}
            </Button>
            <Button variant="destructive" onClick={handleDelete} disabled={deleting}>
              {deleting ? t('common.deleting') : t('common.delete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
