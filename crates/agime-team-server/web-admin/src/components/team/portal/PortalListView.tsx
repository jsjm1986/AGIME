import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Globe, Copy, Check, Trash2 } from 'lucide-react';
import { Button } from '../../ui/button';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { portalApi, type PortalSummary } from '../../../api/portal';
import { agentApi, type TeamAgent } from '../../../api/agent';
import { CreatePortalDialog } from './CreatePortalDialog';
import { useToast } from '../../../contexts/ToastContext';
import { StatusBadge, PORTAL_STATUS_MAP } from '../../ui/status-badge';
import { classifyPortalServiceAgent, getPortalServiceBindingBadgeMeta } from './serviceAgentBinding';
import { copyText } from '../../../utils/clipboard';

interface PortalListViewProps {
  teamId: string;
  canManage: boolean;
  onSelect: (portalId: string) => void;
  domain?: 'ecosystem' | 'avatar';
}

export function PortalListView({ teamId, canManage, onSelect, domain = 'ecosystem' }: PortalListViewProps) {
  const { t } = useTranslation();
  const { addToast } = useToast();
  const [portals, setPortals] = useState<PortalSummary[]>([]);
  const [agents, setAgents] = useState<TeamAgent[]>([]);
  const [loading, setLoading] = useState(true);
  const [createOpen, setCreateOpen] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [portalBaseUrl, setPortalBaseUrl] = useState<string | null>(null);

  const load = async () => {
    try {
      setLoading(true);
      const res = await portalApi.list(teamId, 1, 200, domain);
      setPortals(res.items);
      setPortalBaseUrl(res.portalBaseUrl ?? null);
    } catch {
      addToast('error', t('ecosystem.loadError'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, [teamId, domain]);
  useEffect(() => {
    agentApi.listAgents(teamId).then(res => setAgents(res.items || [])).catch(() => {});
  }, [teamId]);

  const copyUrl = async (url: string, id: string) => {
    if (await copyText(url)) {
      setCopiedId(id);
      setTimeout(() => setCopiedId(null), 2000);
    }
  };

  const handleDelete = (portal: PortalSummary, e: React.MouseEvent) => {
    e.stopPropagation();
    setDeleteTarget(portal.id);
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    try {
      await portalApi.delete(teamId, deleteTarget);
      addToast('success', t('ecosystem.deleteSuccess'));
      await load();
    } catch (err: any) {
      addToast('error', err?.message || t('ecosystem.operationError'));
    } finally {
      setDeleteTarget(null);
    }
  };

  const getBindingModeMeta = (portal: PortalSummary) => {
    const sourceId = portal.serviceAgentId || portal.agentId || portal.codingAgentId || null;
    const agent = agents.find(item => item.id === sourceId) || null;
    const mode = classifyPortalServiceAgent(agent);
    return getPortalServiceBindingBadgeMeta(t, mode);
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
        <h3 className="text-lg font-semibold">{t('ecosystem.noPortals')}</h3>
        <p className="text-sm text-muted-foreground max-w-md">{t('ecosystem.noPortalsHint')}</p>
        {canManage && (
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="w-4 h-4 mr-2" />
            {t('ecosystem.createPortal')}
          </Button>
        )}
        <CreatePortalDialog
          open={createOpen}
          onOpenChange={setCreateOpen}
          teamId={teamId}
          portalBaseUrl={portalBaseUrl}
          onCreated={load}
        />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold">{t('ecosystem.title')}</h2>
        {canManage && (
          <Button size="sm" onClick={() => setCreateOpen(true)}>
            <Plus className="w-4 h-4 mr-1" />
            {t('ecosystem.createPortal')}
          </Button>
        )}
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {portals.map((p) => {
          const targetUrl = p.publicUrl || p.testPublicUrl || p.previewUrl || '';
          const showTestCopy =
            !!p.publicUrl &&
            !!p.testPublicUrl &&
            p.publicUrl !== p.testPublicUrl;
          const bindingModeMeta = domain === 'ecosystem' ? getBindingModeMeta(p) : null;
          return (
            <div
              key={p.id}
              className="border rounded-lg p-4 hover:border-primary/50 cursor-pointer transition-colors"
              onClick={() => onSelect(p.id)}
            >
              <div className="flex items-start justify-between mb-2 gap-2">
                <h3 className="font-medium truncate min-w-0">{p.name}</h3>
                <div className="flex items-center gap-1 shrink-0">
                  <StatusBadge status={PORTAL_STATUS_MAP[p.status]}>
                    {t(`ecosystem.status.${p.status}`)}
                  </StatusBadge>
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
              {bindingModeMeta && (
                <div className="mb-3 flex items-center gap-2">
                  <span className="text-[11px] text-muted-foreground">
                    {t('ecosystem.serviceBindingSummaryLabel', '服务模式')}
                  </span>
                  <span className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-medium ${bindingModeMeta.className}`}>
                    {bindingModeMeta.label}
                  </span>
                </div>
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
                      title={t('ecosystem.copyTestUrl', 'Copy test URL (IP:port)')}
                    >
                      {copiedId === `${p.id}-test` ? <Check className="w-3 h-3" /> : 'IP'}
                    </button>
                  )}
                  <button
                    className="flex items-center gap-1 hover:text-foreground"
                    onClick={(e) => { e.stopPropagation(); if (targetUrl) copyUrl(targetUrl, p.id); }}
                    disabled={!targetUrl}
                  >
                    {copiedId === p.id ? <Check className="w-3 h-3" /> : <Copy className="w-3 h-3" />}
                    {copiedId === p.id ? t('ecosystem.copiedUrl') : t('ecosystem.copyUrl')}
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
        portalBaseUrl={portalBaseUrl}
        onCreated={load}
      />
      <ConfirmDialog
        open={!!deleteTarget}
        onOpenChange={(open) => { if (!open) setDeleteTarget(null); }}
        title={t('ecosystem.deleteConfirm')}
        variant="destructive"
        onConfirm={confirmDelete}
      />
    </div>
  );
}

