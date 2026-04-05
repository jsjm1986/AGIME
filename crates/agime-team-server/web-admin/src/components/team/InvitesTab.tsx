import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ExternalLink, Link2, Trash2, Check, KeyRound } from 'lucide-react';
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
import { formatDate } from '../../utils/format';
import { buildInviteUrl } from '../../utils/navigation';
import { copyText } from '../../utils/clipboard';

interface InvitesTabProps {
  teamId: string;
}

export function InvitesTab({ teamId }: InvitesTabProps) {
  const { t } = useTranslation();
  const [invites, setInvites] = useState<TeamInvite[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
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

  const handleCopyLink = async (code: string) => {
    const url = buildInviteUrl(code);
    if (await copyText(url)) {
      setCopiedKey(`${code}:link`);
      setTimeout(() => setCopiedKey(null), 2000);
      return;
    }
    setError(t('common.error'));
  };

  const handleCopyCode = async (code: string) => {
    if (await copyText(code)) {
      setCopiedKey(`${code}:code`);
      setTimeout(() => setCopiedKey(null), 2000);
      return;
    }
    setError(t('common.error'));
  };

  const handleOpenLink = (code: string) => {
    window.open(buildInviteUrl(code), '_blank', 'noopener,noreferrer');
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
          <TableHead>{t('teams.invite.linkId')}</TableHead>
          <TableHead>{t('teams.invite.inviteeEmail')}</TableHead>
          <TableHead>{t('teams.invite.link')}</TableHead>
          <TableHead>{t('teams.invite.role')}</TableHead>
          <TableHead>{t('teams.invite.createdBy')}</TableHead>
          <TableHead>{t('teams.invite.expires')}</TableHead>
          <TableHead>{t('teams.invite.uses')}</TableHead>
          <TableHead className="w-[180px]">{t('common.actions')}</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {invites.map((invite) => (
          <TableRow key={invite.id} className={isExpired(invite.expiresAt) ? 'opacity-50' : ''}>
            <TableCell className="font-mono text-xs">{invite.id}</TableCell>
            <TableCell className="max-w-[220px]">
              {invite.isOpenInvite ? (
                <Badge variant="outline">{t('teams.invite.openInviteBadge')}</Badge>
              ) : (
                <div className="truncate text-xs text-[hsl(var(--muted-foreground))]">
                  {invite.inviteeEmail}
                </div>
              )}
            </TableCell>
            <TableCell className="max-w-[360px]">
              <div className="truncate font-mono text-xs text-[hsl(var(--muted-foreground))]">
                {buildInviteUrl(invite.id)}
              </div>
            </TableCell>
            <TableCell>
              <Badge variant={invite.role === 'admin' ? 'secondary' : 'outline'}>
                {t(`teams.roles.${invite.role}`)}
              </Badge>
            </TableCell>
            <TableCell>{invite.createdBy}</TableCell>
            <TableCell>
              {invite.expiresAt
                ? formatDate(invite.expiresAt)
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
                  onClick={() => handleOpenLink(invite.id)}
                  title={t('teams.invite.openLink')}
                >
                  <ExternalLink className="h-4 w-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleCopyLink(invite.id)}
                  disabled={isExpired(invite.expiresAt)}
                  title={t('teams.invite.copyLink')}
                >
                  {copiedKey === `${invite.id}:link` ? (
                    <Check className="h-4 w-4" />
                  ) : (
                    <Link2 className="h-4 w-4" />
                  )}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleCopyCode(invite.id)}
                  title={t('teams.invite.copyCode')}
                >
                  {copiedKey === `${invite.id}:code` ? (
                    <Check className="h-4 w-4" />
                  ) : (
                    <KeyRound className="h-4 w-4" />
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
