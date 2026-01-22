import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Link, Trash2, Copy, CheckCircle, Clock, Loader2 } from 'lucide-react';
import { Button } from '../../ui/button';
import {
    Dialog,
    DialogContent,
    DialogHeader,
    DialogTitle,
    DialogDescription,
} from '../../ui/dialog';
import { listInvites, deleteInvite } from '../api';
import type { TeamInvite } from '../types';

interface InviteListDialogProps {
    open: boolean;
    onClose: () => void;
    teamId: string;
    teamName: string;
}

export const InviteListDialog: React.FC<InviteListDialogProps> = ({
    open,
    onClose,
    teamId,
    teamName,
}) => {
    const { t } = useTranslation('team');
    const [invites, setInvites] = useState<TeamInvite[]>([]);
    const [isLoading, setIsLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [copiedCode, setCopiedCode] = useState<string | null>(null);
    const [deletingCode, setDeletingCode] = useState<string | null>(null);

    const loadInvites = async () => {
        setIsLoading(true);
        setError(null);
        try {
            const result = await listInvites(teamId);
            setInvites(result.invites);
        } catch (e) {
            setError(e instanceof Error ? e.message : t('invite.loadError', '加载邀请列表失败'));
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        if (open) {
            loadInvites();
        }
    }, [open, teamId]);

    const handleCopy = async (invite: TeamInvite) => {
        try {
            await navigator.clipboard.writeText(invite.url);
            setCopiedCode(invite.code);
            setTimeout(() => setCopiedCode(null), 2000);
        } catch (e) {
            console.error('Failed to copy:', e);
        }
    };

    const handleDelete = async (code: string) => {
        setDeletingCode(code);
        try {
            await deleteInvite(teamId, code);
            await loadInvites(); // Reload list
        } catch (e) {
            setError(e instanceof Error ? e.message : t('invite.deleteError', '删除邀请失败'));
        } finally {
            setDeletingCode(null);
        }
    };

    const isExpired = (expiresAt?: string) => {
        if (!expiresAt) return false;
        return new Date(expiresAt) < new Date();
    };

    const isMaxUsed = (invite: TeamInvite) => {
        if (!invite.maxUses) return false;
        return invite.usedCount >= invite.maxUses;
    };

    const getStatusColor = (invite: TeamInvite) => {
        if (isExpired(invite.expiresAt)) return 'text-red-600 dark:text-red-400';
        if (isMaxUsed(invite)) return 'text-orange-600 dark:text-orange-400';
        return 'text-green-600 dark:text-green-400';
    };

    const getStatusText = (invite: TeamInvite) => {
        if (isExpired(invite.expiresAt)) return t('invite.expired', '已过期');
        if (isMaxUsed(invite)) return t('invite.maxUsed', '已用完');
        return t('invite.active', '有效');
    };

    return (
        <Dialog open={open} onOpenChange={onClose}>
            <DialogContent className="sm:max-w-[700px] max-h-[80vh] overflow-y-auto">
                <DialogHeader>
                    <DialogTitle className="flex items-center gap-2">
                        <Link size={20} />
                        {t('invite.listTitle', '团队邀请列表')}
                    </DialogTitle>
                    <DialogDescription>
                        {teamName} - {t('invite.listDescription', '管理所有活跃和过期的邀请链接')}
                    </DialogDescription>
                </DialogHeader>

                <div className="space-y-4 mt-4">
                    {isLoading ? (
                        <div className="flex items-center justify-center py-8">
                            <Loader2 size={24} className="animate-spin text-teal-500" />
                        </div>
                    ) : error ? (
                        <div className="p-4 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400">
                            {error}
                        </div>
                    ) : invites.length === 0 ? (
                        <div className="text-center py-8 text-text-muted">
                            {t('invite.noInvites', '暂无邀请')}
                        </div>
                    ) : (
                        <div className="space-y-3">
                            {invites.map((invite) => (
                                <div
                                    key={invite.code}
                                    className="p-4 border border-border-subtle rounded-lg hover:bg-background-muted transition-colors"
                                >
                                    <div className="flex items-start justify-between gap-3">
                                        <div className="flex-1 min-w-0">
                                            {/* Status and Role */}
                                            <div className="flex items-center gap-2 mb-2">
                                                <span className={`text-sm font-medium ${getStatusColor(invite)}`}>
                                                    {getStatusText(invite)}
                                                </span>
                                                <span className="text-xs text-text-muted">•</span>
                                                <span className="text-sm text-text-muted">
                                                    {t('invite.role', '角色')}: <strong>{invite.role}</strong>
                                                </span>
                                            </div>

                                            {/* Invite URL */}
                                            <div className="flex items-center gap-2 mb-2">
                                                <code className="flex-1 text-xs bg-background-muted px-2 py-1 rounded truncate">
                                                    {invite.url}
                                                </code>
                                                <Button
                                                    onClick={() => handleCopy(invite)}
                                                    variant="outline"
                                                    size="sm"
                                                    className="shrink-0"
                                                >
                                                    {copiedCode === invite.code ? (
                                                        <>
                                                            <CheckCircle size={14} className="mr-1" />
                                                            {t('invite.copied', '已复制')}
                                                        </>
                                                    ) : (
                                                        <>
                                                            <Copy size={14} className="mr-1" />
                                                            {t('invite.copy', '复制')}
                                                        </>
                                                    )}
                                                </Button>
                                            </div>

                                            {/* Details */}
                                            <div className="flex items-center gap-4 text-xs text-text-muted">
                                                <span>
                                                    {t('invite.uses', '使用次数')}: {invite.usedCount}
                                                    {invite.maxUses ? `/${invite.maxUses}` : '/∞'}
                                                </span>
                                                {invite.expiresAt && (
                                                    <>
                                                        <span>•</span>
                                                        <span className="flex items-center gap-1">
                                                            <Clock size={12} />
                                                            {t('invite.expires', '过期')}: {new Date(invite.expiresAt).toLocaleString()}
                                                        </span>
                                                    </>
                                                )}
                                            </div>
                                        </div>

                                        {/* Delete Button */}
                                        <Button
                                            onClick={() => handleDelete(invite.code)}
                                            variant="destructive"
                                            size="sm"
                                            disabled={deletingCode === invite.code}
                                        >
                                            {deletingCode === invite.code ? (
                                                <Loader2 size={14} className="animate-spin" />
                                            ) : (
                                                <Trash2 size={14} />
                                            )}
                                        </Button>
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}

                    {/* Footer */}
                    <div className="flex justify-between items-center pt-4 border-t border-border-subtle">
                        <div className="text-sm text-text-muted">
                            {t('invite.total', '共')} {invites.length} {t('invite.invites', '个邀请')}
                        </div>
                        <Button onClick={onClose} variant="outline">
                            {t('common.close', '关闭')}
                        </Button>
                    </div>
                </div>
            </DialogContent>
        </Dialog>
    );
};

export default InviteListDialog;
