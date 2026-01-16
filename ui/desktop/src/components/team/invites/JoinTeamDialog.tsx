import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { UserPlus, Users, Loader2, CheckCircle, XCircle, X } from 'lucide-react';
import { Button } from '../../ui/button';
import { ValidateInviteResponse, AcceptInviteResponse } from '../types';
import { validateInvite, acceptInvite } from '../api';

interface JoinTeamDialogProps {
    open: boolean;
    onClose: () => void;
    onSuccess?: (teamId: string) => void;
    initialCode?: string;
}

type Step = 'input' | 'validating' | 'preview' | 'joining' | 'success' | 'error';

const JoinTeamDialog: React.FC<JoinTeamDialogProps> = ({
    open,
    onClose,
    onSuccess,
    initialCode,
}) => {
    const { t } = useTranslation('team');

    const [step, setStep] = useState<Step>('input');
    const [inviteCode, setInviteCode] = useState(initialCode || '');
    const [displayName, setDisplayName] = useState('');
    const [inviteInfo, setInviteInfo] = useState<ValidateInviteResponse | null>(null);
    const [, setJoinResult] = useState<AcceptInviteResponse | null>(null);
    const [error, setError] = useState<string | null>(null);

    // Parse invite code from URL or raw code
    const parseInviteCode = (input: string): string => {
        const trimmed = input.trim();

        // Try to extract code from URL
        const urlMatch = trimmed.match(/\/join\/([a-zA-Z0-9]+)/);
        if (urlMatch) {
            return urlMatch[1];
        }

        // Try to extract code from full URL
        try {
            const url = new URL(trimmed);
            const pathParts = url.pathname.split('/');
            const joinIndex = pathParts.indexOf('join');
            if (joinIndex !== -1 && pathParts[joinIndex + 1]) {
                return pathParts[joinIndex + 1];
            }
        } catch {
            // Not a URL, use as-is
        }

        return trimmed;
    };

    const handleValidate = async () => {
        const code = parseInviteCode(inviteCode);
        if (!code) {
            setError(t('join.invalidCode', 'Please enter a valid invite code or link'));
            return;
        }

        setStep('validating');
        setError(null);

        try {
            const result = await validateInvite(code);
            setInviteInfo(result);

            if (result.valid) {
                setStep('preview');
            } else {
                setError(result.error || t('join.invalidInvite', 'This invite is invalid or has expired'));
                setStep('error');
            }
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Failed to validate invite');
            setStep('error');
        }
    };

    const handleJoin = async () => {
        const code = parseInviteCode(inviteCode);
        if (!code || !displayName.trim()) return;

        setStep('joining');
        setError(null);

        try {
            const result = await acceptInvite(code, displayName.trim());
            setJoinResult(result);

            if (result.success) {
                setStep('success');
                if (onSuccess && result.teamId) {
                    onSuccess(result.teamId);
                }
            } else {
                setError(result.error || t('join.failed', 'Failed to join team'));
                setStep('error');
            }
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Failed to join team');
            setStep('error');
        }
    };

    const handleClose = () => {
        setStep('input');
        setInviteCode('');
        setDisplayName('');
        setInviteInfo(null);
        setJoinResult(null);
        setError(null);
        onClose();
    };

    const handleRetry = () => {
        setStep('input');
        setError(null);
    };

    // Validate immediately if initialCode is provided
    useEffect(() => {
        if (open && initialCode) {
            handleValidate();
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open, initialCode]);

    if (!open) return null;

    return (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-background-default rounded-lg shadow-xl w-full max-w-md mx-4">
                {/* Header */}
                <div className="flex items-center justify-between p-4 border-b border-border-subtle">
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/30">
                            <UserPlus size={20} className="text-blue-600 dark:text-blue-400" />
                        </div>
                        <h2 className="text-lg font-semibold text-text-default">
                            {t('join.title', 'Join Team')}
                        </h2>
                    </div>
                    <button onClick={handleClose} className="p-1 rounded hover:bg-background-muted">
                        <X size={20} className="text-text-muted" />
                    </button>
                </div>

                {/* Content */}
                <div className="p-4">
                    {step === 'input' && (
                        <div className="space-y-4">
                            <div>
                                <label className="block text-sm font-medium text-text-default mb-1">
                                    {t('join.codeLabel', 'Invite Link or Code')}
                                </label>
                                <input
                                    type="text"
                                    value={inviteCode}
                                    onChange={(e) => setInviteCode(e.target.value)}
                                    placeholder={t('join.codePlaceholder', 'Paste invite link or code...')}
                                    className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500"
                                    autoFocus
                                />
                                <p className="text-xs text-text-muted mt-1">
                                    {t('join.codeHint', 'e.g., https://team.example.com/join/abc123 or abc123')}
                                </p>
                            </div>
                        </div>
                    )}

                    {step === 'validating' && (
                        <div className="flex flex-col items-center justify-center py-8">
                            <Loader2 size={32} className="text-teal-500 animate-spin mb-4" />
                            <p className="text-text-muted">{t('join.validating', 'Validating invite...')}</p>
                        </div>
                    )}

                    {step === 'preview' && inviteInfo && (
                        <div className="space-y-4">
                            <div className="p-4 bg-teal-50 dark:bg-teal-900/20 rounded-lg">
                                <div className="flex items-center gap-3 mb-2">
                                    <Users size={24} className="text-teal-600 dark:text-teal-400" />
                                    <div>
                                        <h3 className="font-medium text-text-default">{inviteInfo.teamName}</h3>
                                        {inviteInfo.teamDescription && (
                                            <p className="text-sm text-text-muted">{inviteInfo.teamDescription}</p>
                                        )}
                                    </div>
                                </div>
                                {inviteInfo.inviterName && (
                                    <p className="text-sm text-text-muted">
                                        {t('join.invitedBy', 'Invited by: {{name}}', { name: inviteInfo.inviterName })}
                                    </p>
                                )}
                                {inviteInfo.role && (
                                    <p className="text-sm text-text-muted">
                                        {t('join.roleInfo', 'You will join as: {{role}}', { role: inviteInfo.role })}
                                    </p>
                                )}
                            </div>

                            <div>
                                <label className="block text-sm font-medium text-text-default mb-1">
                                    {t('join.displayName', 'Your Display Name')}
                                </label>
                                <input
                                    type="text"
                                    value={displayName}
                                    onChange={(e) => setDisplayName(e.target.value)}
                                    placeholder={t('join.displayNamePlaceholder', 'Enter your name...')}
                                    className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500"
                                />
                            </div>
                        </div>
                    )}

                    {step === 'joining' && (
                        <div className="flex flex-col items-center justify-center py-8">
                            <Loader2 size={32} className="text-teal-500 animate-spin mb-4" />
                            <p className="text-text-muted">{t('join.joining', 'Joining team...')}</p>
                        </div>
                    )}

                    {step === 'success' && (
                        <div className="flex flex-col items-center justify-center py-8">
                            <CheckCircle size={48} className="text-green-500 mb-4" />
                            <h3 className="text-lg font-medium text-text-default mb-2">
                                {t('join.success', 'Welcome to the team!')}
                            </h3>
                            <p className="text-text-muted text-center">
                                {t('join.successDesc', 'You have successfully joined {{teamName}}', {
                                    teamName: inviteInfo?.teamName || 'the team',
                                })}
                            </p>
                        </div>
                    )}

                    {step === 'error' && (
                        <div className="flex flex-col items-center justify-center py-8">
                            <XCircle size={48} className="text-red-500 mb-4" />
                            <h3 className="text-lg font-medium text-text-default mb-2">
                                {t('join.errorTitle', 'Unable to Join')}
                            </h3>
                            <p className="text-text-muted text-center mb-4">{error}</p>
                            <Button variant="outline" onClick={handleRetry}>
                                {t('join.tryAgain', 'Try Again')}
                            </Button>
                        </div>
                    )}
                </div>

                {/* Footer */}
                {(step === 'input' || step === 'preview') && (
                    <div className="flex justify-end gap-3 p-4 border-t border-border-subtle">
                        <Button variant="outline" onClick={handleClose}>
                            {t('cancel', 'Cancel')}
                        </Button>
                        {step === 'input' ? (
                            <Button onClick={handleValidate} disabled={!inviteCode.trim()}>
                                {t('join.continue', 'Continue')}
                            </Button>
                        ) : (
                            <Button onClick={handleJoin} disabled={!displayName.trim()}>
                                {t('join.joinButton', 'Join Team')}
                            </Button>
                        )}
                    </div>
                )}

                {step === 'success' && (
                    <div className="flex justify-end p-4 border-t border-border-subtle">
                        <Button onClick={handleClose}>
                            {t('common.done', 'Done')}
                        </Button>
                    </div>
                )}
            </div>
        </div>
    );
};

export default JoinTeamDialog;
