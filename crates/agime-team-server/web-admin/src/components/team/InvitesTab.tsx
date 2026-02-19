import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Copy, Trash2, Check } from 'lucide-react';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../ui/table';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { apiClient } from '../../api/client';
import type { TeamInvite } from '../../api/types';

interface InvitesTabProps {
  teamId: string;
}

export function InvitesTab({ teamId }: InvitesTabProps) {
  const { t } = useTranslation();
  const [invites, setInvites] = useState<TeamInvite[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [copiedCode, setCopiedCode] = useState<string | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<string | null>(null);

  const loadInvites = async () => {
    try {
      setLoading(true);
      const response = await apiClient.getInvites(teamId);
      setInvites(response.invites);
      setError('');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadInvites();
  }, [teamId]);

  const handleCopy = async (code: string) => {
    const url = `${window.location.origin}/join/${code}`;
    await navigator.clipboard.writeText(url);
    setCopiedCode(code);
    setTimeout(() => setCopiedCode(null), 2000);
  };

  const handleRevoke = (code: string) => {
    setRevokeTarget(code);
  };

  const confirmRevoke = async () => {
    if (!revokeTarget) return;
    try {
      await apiClient.revokeInvite(teamId, revokeTarget);
      loadInvites();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setRevokeTarget(null);
    }
  };

  const isExpired = (expiresAt: string | null) => {
    if (!expiresAt) return false;
    return new Date(expiresAt) < new Date();
  };

  if (loading) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('common.loading')}</p>;
  }

  if (error) {
    return <p className="text-center py-8 text-[hsl(var(--destructive))]">{error}</p>;
  }

  if (invites.length === 0) {
    return <p className="text-center py-8 text-[hsl(var(--muted-foreground))]">{t('teams.invite.noInvites')}</p>;
  }

  return (
    <>
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>{t('teams.invite.code')}</TableHead>
          <TableHead>{t('teams.invite.role')}</TableHead>
          <TableHead>{t('teams.invite.createdBy')}</TableHead>
          <TableHead>{t('teams.invite.expires')}</TableHead>
          <TableHead>{t('teams.invite.uses')}</TableHead>
          <TableHead className="w-[100px]">{t('common.actions')}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {invites.map((invite) => (
          <TableRow key={invite.id} className={isExpired(invite.expiresAt) ? 'opacity-50' : ''}>
            <TableCell className="font-mono text-sm">{invite.id}</TableCell>
            <TableCell>
              <Badge variant={invite.role === 'admin' ? 'secondary' : 'outline'}>
                {t(`teams.roles.${invite.role}`)}
              </Badge>
            </TableCell>
            <TableCell>{invite.createdBy}</TableCell>
            <TableCell>
              {invite.expiresAt
                ? new Date(invite.expiresAt).toLocaleDateString()
                : t('teams.invite.never')}
            </TableCell>
            <TableCell>
              {invite.usedCount}
              {invite.maxUses && ` / ${invite.maxUses}`}
            </TableCell>
            <TableCell>
              <div className="flex gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleCopy(invite.id)}
                  disabled={isExpired(invite.expiresAt)}
                >
                  {copiedCode === invite.id ? (
                    <Check className="h-4 w-4" />
                  ) : (
                    <Copy className="h-4 w-4" />
                  )}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleRevoke(invite.id)}
                >
                  <Trash2 className="h-4 w-4" />
                </Button>
              </div>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
    <ConfirmDialog
      open={!!revokeTarget}
      onOpenChange={(open) => { if (!open) setRevokeTarget(null); }}
      title={t('teams.invite.revokeConfirm')}
      variant="destructive"
      onConfirm={confirmRevoke}
    />
    </>
  );
}
