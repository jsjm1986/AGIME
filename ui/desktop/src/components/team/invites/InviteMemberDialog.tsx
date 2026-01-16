import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Link2, Copy, Check, Users, Clock, X } from 'lucide-react';
import { Button } from '../../ui/button';
import { CreateInviteResponse, InviteExpiration, InviteRole } from '../types';
import { createInvite } from '../api';

interface InviteMemberDialogProps {
    open: boolean;
    onClose: () => void;
    teamId: string;
    teamName: string;
}

const InviteMemberDialog: React.FC<InviteMemberDialogProps> = ({
    open,
    onClose,
    teamId,
    teamName,
}) => {
    const { t } = useTranslation('team');

    const [expiresIn, setExpiresIn] = useState<InviteExpiration>('7d');
    const [maxUses, setMaxUses] = useState<number | undefined>(undefined);
    const [role, setRole] = useState<InviteRole>('member');
    const [isCreating, setIsCreating] = useState(false);
    const [invite, setInvite] = useState<CreateInviteResponse | null>(null);
    const [copied, setCopied] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const handleCreate = async () => {
        setIsCreating(true);
        setError(null);

        try {
            const result = await createInvite(teamId, {
                expires_in: expiresIn,
                max_uses: maxUses,
                role,
            });
            setInvite(result);
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Failed to create invite');
        } finally {
            setIsCreating(false);
        }
    };

    const handleCopy = async () => {
        if (!invite) return;

        try {
            await navigator.clipboard.writeText(invite.url);
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        } catch {
            // Fallback for older browsers
            const textArea = document.createElement('textarea');
            textArea.value = invite.url;
            document.body.appendChild(textArea);
            textArea.select();
            document.execCommand('copy');
            document.body.removeChild(textArea);
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        }
    };

    const handleClose = () => {
        setInvite(null);
        setExpiresIn('7d');
        setMaxUses(undefined);
        setRole('member');
        setError(null);
        onClose();
    };

    if (!open) return null;

    return (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-background-default rounded-lg shadow-xl w-full max-w-md mx-4">
                {/* Header */}
                <div className="flex items-center justify-between p-4 border-b border-border-subtle">
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-teal-100 dark:bg-teal-900/30">
                            <Users size={20} className="text-teal-600 dark:text-teal-400" />
                        </div>
                        <div>
                            <h2 className="text-lg font-semibold text-text-default">
                                {t('invite.title', 'Invite Members')}
                            </h2>
                            <p className="text-sm text-text-muted">{teamName}</p>
                        </div>
                    </div>
                    <button onClick={handleClose} className="p-1 rounded hover:bg-background-muted">
                        <X size={20} className="text-text-muted" />
                    </button>
                </div>

                {/* Content */}
                <div className="p-4 space-y-4">
                    {invite ? (
                        // Show invite link
                        <div className="space-y-4">
                            <div className="p-4 bg-teal-50 dark:bg-teal-900/20 rounded-lg">
                                <div className="flex items-center gap-2 mb-2">
                                    <Link2 size={16} className="text-teal-600 dark:text-teal-400" />
                                    <span className="text-sm font-medium text-teal-700 dark:text-teal-300">
                                        {t('invite.linkReady', 'Invite link is ready!')}
                                    </span>
                                </div>
                                <div className="flex items-center gap-2">
                                    <input
                                        type="text"
                                        value={invite.url}
                                        readOnly
                                        className="flex-1 px-3 py-2 text-sm bg-white dark:bg-gray-800 border border-border-subtle rounded-lg"
                                    />
                                    <Button onClick={handleCopy} size="sm">
                                        {copied ? (
                                            <>
                                                <Check size={14} className="mr-1" />
                                                {t('invite.copied', 'Copied!')}
                                            </>
                                        ) : (
                                            <>
                                                <Copy size={14} className="mr-1" />
                                                {t('invite.copy', 'Copy')}
                                            </>
                                        )}
                                    </Button>
                                </div>
                            </div>

                            {invite.expiresAt && (
                                <p className="text-sm text-text-muted flex items-center gap-2">
                                    <Clock size={14} />
                                    {t('invite.expiresAt', 'Expires: {{date}}', {
                                        date: new Date(invite.expiresAt).toLocaleString(),
                                    })}
                                </p>
                            )}

                            {invite.maxUses && (
                                <p className="text-sm text-text-muted">
                                    {t('invite.usesRemaining', 'Uses remaining: {{count}}', {
                                        count: invite.maxUses - invite.usedCount,
                                    })}
                                </p>
                            )}

                            <Button onClick={handleClose} className="w-full">
                                {t('common.done', 'Done')}
                            </Button>
                        </div>
                    ) : (
                        // Create invite form
                        <>
                            {/* Expiration */}
                            <div>
                                <label className="block text-sm font-medium text-text-default mb-2">
                                    {t('invite.expiration', 'Link Expiration')}
                                </label>
                                <div className="grid grid-cols-4 gap-2">
                                    {(['24h', '7d', '30d', 'never'] as InviteExpiration[]).map((option) => (
                                        <button
                                            key={option}
                                            onClick={() => setExpiresIn(option)}
                                            className={`
                        py-2 px-3 text-sm rounded-lg border transition-colors
                        ${expiresIn === option
                                                    ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20 text-teal-700 dark:text-teal-300'
                                                    : 'border-border-subtle hover:border-border-default'
                                                }
                      `}
                                        >
                                            {t(`invite.expires.${option}`, option)}
                                        </button>
                                    ))}
                                </div>
                            </div>

                            {/* Max Uses */}
                            <div>
                                <label className="block text-sm font-medium text-text-default mb-2">
                                    {t('invite.maxUses', 'Max Uses')}
                                </label>
                                <div className="grid grid-cols-4 gap-2">
                                    {[1, 5, 10, undefined].map((option) => (
                                        <button
                                            key={option ?? 'unlimited'}
                                            onClick={() => setMaxUses(option)}
                                            className={`
                        py-2 px-3 text-sm rounded-lg border transition-colors
                        ${maxUses === option
                                                    ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20 text-teal-700 dark:text-teal-300'
                                                    : 'border-border-subtle hover:border-border-default'
                                                }
                      `}
                                        >
                                            {option ?? t('invite.unlimited', 'Unlimited')}
                                        </button>
                                    ))}
                                </div>
                            </div>

                            {/* Role */}
                            <div>
                                <label className="block text-sm font-medium text-text-default mb-2">
                                    {t('invite.role', 'Member Role')}
                                </label>
                                <div className="grid grid-cols-2 gap-2">
                                    {(['member', 'admin'] as InviteRole[]).map((option) => (
                                        <button
                                            key={option}
                                            onClick={() => setRole(option)}
                                            className={`
                        py-2 px-3 text-sm rounded-lg border transition-colors
                        ${role === option
                                                    ? 'border-teal-500 bg-teal-50 dark:bg-teal-900/20 text-teal-700 dark:text-teal-300'
                                                    : 'border-border-subtle hover:border-border-default'
                                                }
                      `}
                                        >
                                            {t(`roles.${option}`, option)}
                                        </button>
                                    ))}
                                </div>
                            </div>

                            {error && (
                                <div className="p-3 bg-red-50 dark:bg-red-900/20 rounded-lg text-sm text-red-600 dark:text-red-400">
                                    {error}
                                </div>
                            )}
                        </>
                    )}
                </div>

                {/* Footer */}
                {!invite && (
                    <div className="flex justify-end gap-3 p-4 border-t border-border-subtle">
                        <Button variant="outline" onClick={handleClose}>
                            {t('cancel', 'Cancel')}
                        </Button>
                        <Button onClick={handleCreate} disabled={isCreating}>
                            {isCreating
                                ? t('invite.creating', 'Creating...')
                                : t('invite.create', 'Create Invite Link')}
                        </Button>
                    </div>
                )}
            </div>
        </div>
    );
};

export default InviteMemberDialog;
