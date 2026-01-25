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
import type { SharedExtension } from '../../api/types';

interface ExtensionsTabProps {
  teamId: string;
  canManage: boolean;
}

export function ExtensionsTab({ teamId, canManage }: ExtensionsTabProps) {
  const { t } = useTranslation();
  const [extensions, setExtensions] = useState<SharedExtension[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [selectedExt, setSelectedExt] = useState<SharedExtension | null>(null);
  const [dialogMode, setDialogMode] = useState<'view' | 'edit'>('view');

  const loadExtensions = async () => {
    try {
      setLoading(true);
      const response = await apiClient.getExtensions(teamId);
      setExtensions(response.extensions);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadExtensions();
  }, [teamId]);

  const handleDelete = async (extId: string) => {
    if (!confirm(t('teams.resource.deleteConfirm'))) return;
    try {
      await apiClient.deleteExtension(extId);
      loadExtensions();
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

  if (extensions.length === 0) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.resource.noExtensions')}</p>;
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
          {extensions.map((ext) => (
            <TableRow key={ext.id}>
              <TableCell className="font-medium">{ext.name}</TableCell>
              <TableCell>{ext.authorId}</TableCell>
              <TableCell>{ext.version}</TableCell>
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
                  >
                    <Eye className="h-4 w-4" />
                  </Button>
                  {canManage && (
                    <>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setSelectedExt(ext);
                          setDialogMode('edit');
                        }}
                      >
                        <Pencil className="h-4 w-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDelete(ext.id)}
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
    </>
  );
}
