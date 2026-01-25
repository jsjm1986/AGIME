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
import type { SharedSkill } from '../../api/types';

interface SkillsTabProps {
  teamId: string;
  canManage: boolean;
}

export function SkillsTab({ teamId, canManage }: SkillsTabProps) {
  const { t } = useTranslation();
  const [skills, setSkills] = useState<SharedSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedSkill, setSelectedSkill] = useState<SharedSkill | null>(null);
  const [dialogMode, setDialogMode] = useState<'view' | 'edit'>('view');

  const loadSkills = async () => {
    try {
      setLoading(true);
      const response = await apiClient.getSkills(teamId);
      setSkills(response.skills);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadSkills();
  }, [teamId]);

  const handleDelete = async (skillId: string) => {
    if (!confirm(t('teams.resource.deleteConfirm'))) return;
    try {
      await apiClient.deleteSkill(skillId);
      loadSkills();
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

  if (skills.length === 0) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.resource.noSkills')}</p>;
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
          {skills.map((skill) => (
            <TableRow key={skill.id}>
              <TableCell className="font-medium">{skill.name}</TableCell>
              <TableCell>{skill.authorId}</TableCell>
              <TableCell>{skill.version}</TableCell>
              <TableCell>{skill.useCount}</TableCell>
              <TableCell>
                <div className="flex gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      setSelectedSkill(skill);
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
                          setSelectedSkill(skill);
                          setDialogMode('edit');
                        }}
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDelete(skill.id)}
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
        open={!!selectedSkill}
        onOpenChange={() => setSelectedSkill(null)}
        resource={selectedSkill}
        resourceType="skill"
        mode={dialogMode}
        onSave={async (data) => {
          if (selectedSkill) {
            await apiClient.updateSkill(selectedSkill.id, data);
            loadSkills();
          }
        }}
      />
    </>
  );
}
