import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Check, ExternalLink, KeyRound, Link2 } from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '../ui/dialog';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../ui/select';
import { apiClient } from '../../api/client';
import type { TeamRole } from '../../api/types';
import { buildInviteUrl } from '../../utils/navigation';
import { copyText } from '../../utils/clipboard';

interface CreateInviteDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  teamId: string;
  onCreated: () => void;
}

type InviteMode = 'email' | 'open';

export function CreateInviteDialog({ open, onOpenChange, teamId, onCreated }: CreateInviteDialogProps) {
  const { t } = useTranslation();
  const [inviteMode, setInviteMode] = useState<InviteMode>('email');
  const [inviteeEmail, setInviteeEmail] = useState('');
  const [role, setRole] = useState<TeamRole>('member');
  const [expiresInDays, setExpiresInDays] = useState('7');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [inviteUrl, setInviteUrl] = useState('');
  const [inviteCode, setInviteCode] = useState('');
  const [createdAsOpenInvite, setCreatedAsOpenInvite] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copiedCode, setCopiedCode] = useState(false);

  const handleCreate = async () => {
    setLoading(true);
    setError('');
    try {
      const isOpenInvite = inviteMode === 'open';
      const response = await apiClient.createInvite(
        teamId,
        isOpenInvite ? undefined : inviteeEmail.trim(),
        isOpenInvite,
        role,
        expiresInDays && expiresInDays !== 'never' ? parseInt(expiresInDays) : undefined,
        isOpenInvite ? 1 : undefined,
      );
      setInviteCode(response.code);
      setInviteeEmail(response.inviteeEmail);
      setCreatedAsOpenInvite(response.isOpenInvite);
      setInviteUrl(buildInviteUrl(response.url));
      onCreated();
    } catch (err) {
      setError(err instanceof Error ? err.message : t('common.error'));
    } finally {
      setLoading(false);
    }
  };

  const handleCopy = async () => {
    if (await copyText(inviteUrl)) {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const handleCopyCode = async () => {
    if (await copyText(inviteCode)) {
      setCopiedCode(true);
      setTimeout(() => setCopiedCode(false), 2000);
    }
  };

  const handleClose = () => {
    setInviteUrl('');
    setInviteCode('');
    setInviteMode('email');
    setInviteeEmail('');
    setRole('member');
    setExpiresInDays('7');
    setCreatedAsOpenInvite(false);
    setError('');
    setCopied(false);
    setCopiedCode(false);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('teams.invite.create')}</DialogTitle>
        </DialogHeader>

        {inviteUrl ? (
          <div className="space-y-4">
            <p className="text-sm text-[hsl(var(--muted-foreground))]">
              {t('teams.invite.created')}
            </p>
            <div className="space-y-1">
              <div className="text-xs font-medium text-[hsl(var(--muted-foreground))]">
                {t('teams.invite.linkLabel')}
              </div>
              <div className="rounded-[12px] border border-[hsl(var(--border))] bg-[hsl(var(--muted))]/25 px-3 py-2 text-xs font-mono break-all">
                {inviteUrl}
              </div>
            </div>
            <div className="space-y-1">
              <div className="text-xs font-medium text-[hsl(var(--muted-foreground))]">
                {t('teams.invite.codeLabel')}
              </div>
              <div className="rounded-[12px] border border-[hsl(var(--border))] bg-[hsl(var(--muted))]/25 px-3 py-2 text-xs font-mono break-all">
                {inviteCode}
              </div>
            </div>
            {createdAsOpenInvite ? (
              <div className="rounded-[12px] border border-[hsl(var(--destructive))]/20 bg-[hsl(var(--destructive))]/5 px-3 py-2 text-xs text-[hsl(var(--destructive))]">
                {t('teams.invite.openInviteCreatedWarning')}
              </div>
            ) : (
              <div className="space-y-1">
                <div className="text-xs font-medium text-[hsl(var(--muted-foreground))]">
                  {t('teams.invite.inviteeEmail')}
                </div>
                <div className="rounded-[12px] border border-[hsl(var(--border))] bg-[hsl(var(--muted))]/25 px-3 py-2 text-xs break-all">
                  {inviteeEmail}
                </div>
              </div>
            )}
            <div className="flex gap-2">
              <Button onClick={() => window.open(inviteUrl, '_blank', 'noopener,noreferrer')} variant="outline">
                <ExternalLink className="h-4 w-4" />
                {t('teams.invite.openLink')}
              </Button>
              <Button onClick={handleCopy} variant="outline">
                {copied ? (
                  <>
                    <Check className="h-4 w-4" />
                    {t('teams.invite.copied')}
                  </>
                ) : (
                  <>
                    <Link2 className="h-4 w-4" />
                    {t('teams.invite.copyLink')}
                  </>
                )}
              </Button>
              <Button onClick={handleCopyCode} variant="outline">
                {copiedCode ? (
                  <>
                    <Check className="h-4 w-4" />
                    {t('teams.invite.copied')}
                  </>
                ) : (
                  <>
                    <KeyRound className="h-4 w-4" />
                    {t('teams.invite.copyCode')}
                  </>
                )}
              </Button>
            </div>
            <DialogFooter>
              <Button onClick={handleClose}>{t('common.done')}</Button>
            </DialogFooter>
          </div>
        ) : (
          <div className="space-y-5 pt-1">
            <div className="space-y-1.5">
              <label className="text-sm font-medium block">{t('teams.invite.mode')}</label>
              <Select
                value={inviteMode}
                onValueChange={(value) => {
                  const mode = value as InviteMode;
                  setInviteMode(mode);
                  setExpiresInDays(mode === 'open' ? '1' : '7');
                  setError('');
                }}
              >
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="email">{t('teams.invite.modeEmail')}</SelectItem>
                  <SelectItem value="open">{t('teams.invite.modeOpen')}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {inviteMode === 'open' ? (
              <div className="rounded-[12px] border border-[hsl(var(--destructive))]/20 bg-[hsl(var(--destructive))]/5 px-3 py-3 text-xs text-[hsl(var(--destructive))]">
                {t('teams.invite.openInviteWarning')}
              </div>
            ) : (
            <div className="space-y-1.5">
              <label className="text-sm font-medium block">{t('teams.invite.inviteeEmail')}</label>
              <Input
                type="email"
                value={inviteeEmail}
                onChange={(e) => setInviteeEmail(e.target.value)}
                placeholder={t('teams.invite.inviteeEmailPlaceholder')}
                className="h-9"
                required
              />
              <p className="text-caption text-muted-foreground/75">{t('teams.invite.inviteeEmailHint')}</p>
            </div>
            )}
            <div className="space-y-1.5">
              <label className="text-sm font-medium block">{t('teams.invite.role')}</label>
              <Select value={role} onValueChange={(v) => setRole(v as TeamRole)}>
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="admin">{t('teams.roles.admin')}</SelectItem>
                  <SelectItem value="member">{t('teams.roles.member')}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <label className="text-sm font-medium block">{t('teams.invite.expiresIn')}</label>
              <Select value={expiresInDays} onValueChange={setExpiresInDays}>
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {inviteMode === 'open' && (
                    <SelectItem value="1">{t('teams.invite.days', { count: 1 })}</SelectItem>
                  )}
                  {inviteMode === 'open' && (
                    <SelectItem value="3">{t('teams.invite.days', { count: 3 })}</SelectItem>
                  )}
                  <SelectItem value="7">{t('teams.invite.days', { count: 7 })}</SelectItem>
                  {inviteMode !== 'open' && (
                    <SelectItem value="30">{t('teams.invite.days', { count: 30 })}</SelectItem>
                  )}
                  {inviteMode !== 'open' && (
                    <SelectItem value="never">{t('teams.invite.never')}</SelectItem>
                  )}
                </SelectContent>
              </Select>
            </div>
            <p className="rounded-[12px] border border-[hsl(var(--border))] bg-[hsl(var(--muted))]/25 px-3 py-2 text-xs text-[hsl(var(--muted-foreground))]">
              {inviteMode === 'open'
                ? t('teams.invite.openInviteLimitHint')
                : t('teams.invite.singleUseHint')}
            </p>
            {error && <p className="text-sm text-[hsl(var(--destructive))]">{error}</p>}
            <DialogFooter className="pt-2">
              <Button variant="outline" onClick={handleClose}>
                {t('common.cancel')}
              </Button>
              <Button
                onClick={handleCreate}
                disabled={loading || (inviteMode === 'email' && !inviteeEmail.trim())}
              >
                {loading ? t('common.creating') : t('common.create')}
              </Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
