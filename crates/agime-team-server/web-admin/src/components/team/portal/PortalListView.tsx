import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Globe, Copy, Check, Trash2 } from 'lucide-react';
import { Button } from '../../ui/button';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { portalApi, type PortalSummary } from '../../../api/portal';
import { CreatePortalDialog } from './CreatePortalDialog';
import { useToast } from '../../../contexts/ToastContext';

interface PortalListViewProps {
  teamId: string;
  canManage: boolean;
  onSelect: (portalId: string) => void;
}

const statusColors: Record<string, string> = {
  draft: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-900/30 dark:text-yellow-400',
  published: 'bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400',
  archived: 'bg-gray-100 text-gray-800 dark:bg-gray-900/30 dark:text-gray-400',
};

export function PortalListView({ teamId, canManage, onSelect }: PortalListViewProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [portals, setPortals] = useState<PortalSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  const load = async () => {
    try {
      setLoading(true);
      const res = await portalApi.list(teamId);
      setPortals(res.items);
    } catch {
      addToast('error', t('laboratory.loadError'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, [teamId]);

  const copyUrl = (url: string, id: string) => {
    navigator.clipboard.writeText(url);
    setCopiedId(id);
    setTimeout(() => setCopiedId(null), 2000);
  };

  const handleDelete = (portal: PortalSummary, e: React.MouseEvent) => {
    e.stopPropagation();
    setDeleteTarget(portal.id);
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    try {
      await portalApi.delete(teamId, deleteTarget);
      addToast('success', t('laboratory.deleteSuccess'));
      await load();
    } catch (err: any) {
      addToast('error', err?.message || t('laboratory.operationError'));
    } finally {
      setDeleteTarget(null);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center py-24">
        <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary" />
      </div>
    );
  }

  if (portals.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-24 gap-4 text-center">
        <Globe className="w-12 h-12 text-muted-foreground" />
        <h3 className="text-lg font-semibold">{t('laboratory.noPortals')}</h3>
        <p className="text-sm text-muted-foreground max-w-md">{t('laboratory.noPortalsHint')}</p>
        {canManage && (
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="w-4 h-4 mr-2" />
            {t('laboratory.createPortal')}
          </Button>
        )}
        <CreatePortalDialog
          open={createOpen}
          onOpenChange={setCreateOpen}
          teamId={teamId}
          onCreated={load}
        />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t('laboratory.title')}</h2>
        {canManage && (
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus className="w-4 h-4 mr-1" />
            {t('laboratory.createPortal')}
          </Button>
        )}
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {portals.map((p) => {
          const targetUrl = p.status === 'published'
            ? (p.publicUrl || p.testPublicUrl || `/p/${p.slug}`)
            : p.previewUrl;
          const showTestCopy =
            p.status === 'published' &&
            !!p.publicUrl &&
            !!p.testPublicUrl &&
            p.publicUrl !== p.testPublicUrl;
          return (
            <div
              key={p.id}
              className="border rounded-lg p-4 hover:border-primary/50 cursor-pointer transition-colors"
              onClick={() => onSelect(p.id)}
            >
              <div className="flex items-start justify-between mb-2 gap-2">
                <h3 className="font-medium truncate min-w-0">{p.name}</h3>
                <div className="flex items-center gap-1 shrink-0">
                  <span className={`text-xs px-2 py-0.5 rounded-full ${statusColors[p.status] || ''}`}>
                    {t(`laboratory.status.${p.status}`)}
                  </span>
                  {canManage && (
                    <button
                      className="p-1 rounded hover:bg-muted text-muted-foreground hover:text-[hsl(var(--destructive))]"
                      onClick={(e) => handleDelete(p, e)}
                      title={t('common.delete')}
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  )}
                </div>
              </div>
              {p.description && (
                <p className="text-sm text-muted-foreground line-clamp-2 mb-3">{p.description}</p>
              )}
              <div className="flex items-center justify-between text-xs text-muted-foreground">
                <span>/p/{p.slug}</span>
                <div className="flex items-center gap-1">
                  {showTestCopy && (
                    <button
                      className="px-1 py-0.5 rounded border border-border hover:text-foreground"
                      onClick={(e) => {
                        e.stopPropagation();
                        copyUrl(p.testPublicUrl as string, `${p.id}-test`);
                      }}
                      title={t('laboratory.copyTestUrl', 'Copy test URL (IP:port)')}
                    >
                      {copiedId === `${p.id}-test` ? <Check className="w-3 h-3" /> : 'IP'}
                    </button>
                  )}
                  <button
                    className="flex items-center gap-1 hover:text-foreground"
                    onClick={(e) => { e.stopPropagation(); copyUrl(targetUrl, p.id); }}
                  >
                    {copiedId === p.id ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                    {copiedId === p.id ? t('laboratory.copiedUrl') : t('laboratory.copyUrl')}
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <CreatePortalDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        teamId={teamId}
        onCreated={load}
      />
      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
        title={t('laboratory.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDelete}
      />
    </div>
  );
}
