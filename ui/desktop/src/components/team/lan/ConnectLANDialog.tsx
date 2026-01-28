import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Monitor, CheckCircle, XCircle, Loader2, Eye, EyeOff, X } from 'lucide-react';
import { Button } from '../../ui/button';
import type { DataSource } from '../sources/types';
import { sourceManager } from '../sources/sourceManager';
import { authAdapter, storeCredential } from '../auth/authAdapter';

interface ConnectLANDialogProps {
    open: boolean;
    onClose: () => void;
    onSuccess: (source: DataSource) => void;
}

type Step = 'input' | 'testing' | 'success' | 'error';

const ConnectLANDialog: React.FC<ConnectLANDialogProps> = ({
    open,
    onClose,
    onSuccess,
}) => {
    const { t } = useTranslation('team');

    const [step, setStep] = useState<Step>('input');
    const [host, setHost] = useState('');
    const [port, setPort] = useState('7778');
    const [secretKey, setSecretKey] = useState('');
    const [name, setName] = useState('');
    const [myNickname, setMyNickname] = useState('');
    const [showSecretKey, setShowSecretKey] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [testResult, setTestResult] = useState<{ teamsCount?: number } | null>(null);

    const resetForm = () => {
        setStep('input');
        setHost('');
        setPort('7778');
        setSecretKey('');
        setName('');
        setMyNickname('');
        setError(null);
        setTestResult(null);
        setShowSecretKey(false);
    };

    const handleClose = () => {
        resetForm();
        onClose();
    };

    const handleTestConnection = async () => {
        if (!host.trim() || !secretKey.trim()) {
            setError(t('lan.requiredFields', 'Host and Secret Key are required'));
            return;
        }

        const portNum = parseInt(port, 10);
        if (isNaN(portNum) || portNum < 1 || portNum > 65535) {
            setError(t('lan.invalidPort', 'Invalid port number'));
            return;
        }

        setStep('testing');
        setError(null);

        const url = `http://${host.trim()}:${portNum}`;
        const result = await authAdapter.testConnection(url, 'secret-key', secretKey.trim());

        if (result.success) {
            setTestResult({ teamsCount: result.teamsCount });
            setStep('success');

            // Auto-generate name if not provided
            if (!name.trim()) {
                setName(host);
            }
        } else {
            setError(result.error || t('lan.connectionFailed', 'Connection failed'));
            setStep('error');
        }
    };

    const handleSave = () => {
        try {
            const portNum = parseInt(port, 10);
            const url = `http://${host.trim()}:${portNum}`;
            const sourceId = `lan-${Date.now()}`;

            // Store credential securely
            storeCredential(sourceId, secretKey.trim());

            // Create DataSource
            const source: DataSource = {
                id: sourceId,
                type: 'lan',
                name: name.trim() || host,
                status: 'online',
                connection: {
                    url,
                    authType: 'secret-key',
                    credentialRef: sourceId,
                },
                capabilities: {
                    canCreate: false,
                    canSync: true,
                    supportsOffline: false,
                    canManageTeams: false,
                    canInviteMembers: false,
                },
                teamsCount: testResult?.teamsCount,
                createdAt: new Date().toISOString(),
            };

            // Register with SourceManager
            sourceManager.registerSource(source);

            onSuccess(source);
            handleClose();
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Failed to save connection');
            setStep('error');
        }
    };

    if (!open) return null;

    return (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-background-default rounded-lg shadow-xl w-full max-w-md mx-4">
                {/* Header */}
                <div className="flex items-center justify-between p-4 border-b border-border-subtle">
                    <div className="flex items-center gap-3">
                        <div className="p-2 rounded-lg bg-green-100 dark:bg-green-900/30">
                            <Monitor size={20} className="text-green-600 dark:text-green-400" />
                        </div>
                        <h2 className="text-lg font-semibold text-text-default">
                            {t('lan.connectTitle', 'Connect to LAN Device')}
                        </h2>
                    </div>
                    <button onClick={handleClose} className="p-1 rounded hover:bg-background-muted">
                        <X size={20} className="text-text-muted" />
                    </button>
                </div>

                {/* Content */}
                <div className="p-4 space-y-4">
                    {/* Host */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('lan.host', 'Host Address')} *
                        </label>
                        <input
                            type="text"
                            value={host}
                            onChange={(e) => setHost(e.target.value)}
                            placeholder="192.168.1.100"
                            disabled={step === 'testing'}
                            className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-green-500 disabled:opacity-50"
                        />
                    </div>

                    {/* Port */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('lan.port', 'Port')}
                        </label>
                        <input
                            type="number"
                            value={port}
                            onChange={(e) => setPort(e.target.value)}
                            placeholder="7778"
                            disabled={step === 'testing'}
                            className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-green-500 disabled:opacity-50"
                        />
                    </div>

                    {/* Secret Key */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('lan.secretKey', 'Secret Key')} *
                        </label>
                        <div className="relative">
                            <input
                                type={showSecretKey ? 'text' : 'password'}
                                value={secretKey}
                                onChange={(e) => setSecretKey(e.target.value)}
                                placeholder={t('lan.secretKeyPlaceholder', 'Enter the secret key from the other device')}
                                disabled={step === 'testing'}
                                className="w-full px-3 py-2 pr-10 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-green-500 disabled:opacity-50"
                            />
                            <button
                                type="button"
                                onClick={() => setShowSecretKey(!showSecretKey)}
                                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-text-muted hover:text-text-default"
                            >
                                {showSecretKey ? <EyeOff size={16} /> : <Eye size={16} />}
                            </button>
                        </div>
                        <p className="text-xs text-text-muted mt-1">
                            {t('lan.secretKeyHint', 'Ask the device owner to share their Secret Key')}
                        </p>
                    </div>

                    {/* Name */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('lan.deviceName', 'Device Name')}
                        </label>
                        <input
                            type="text"
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            placeholder={t('lan.deviceNamePlaceholder', "e.g., John's Workstation")}
                            disabled={step === 'testing'}
                            className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-green-500 disabled:opacity-50"
                        />
                    </div>

                    {/* My Nickname */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('lan.myNickname', 'Your Nickname')}
                        </label>
                        <input
                            type="text"
                            value={myNickname}
                            onChange={(e) => setMyNickname(e.target.value)}
                            placeholder={t('lan.myNicknamePlaceholder', 'How you appear to others')}
                            disabled={step === 'testing'}
                            className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-green-500 disabled:opacity-50"
                        />
                    </div>

                    {/* Status messages */}
                    {step === 'testing' && (
                        <div className="flex items-center gap-2 p-3 bg-green-50 dark:bg-green-900/20 rounded-lg">
                            <Loader2 size={16} className="text-green-500 animate-spin" />
                            <span className="text-sm text-green-700 dark:text-green-300">
                                {t('lan.testing', 'Testing connection...')}
                            </span>
                        </div>
                    )}

                    {step === 'success' && testResult && (
                        <div className="p-3 bg-green-50 dark:bg-green-900/20 rounded-lg">
                            <div className="flex items-center gap-2 mb-2">
                                <CheckCircle size={16} className="text-green-500" />
                                <span className="text-sm font-medium text-green-700 dark:text-green-300">
                                    {t('lan.connected', 'Connected successfully!')}
                                </span>
                            </div>
                            {testResult.teamsCount !== undefined && (
                                <p className="text-sm text-green-600 dark:text-green-400">
                                    {t('lan.teamsAvailable', '{{count}} teams available', { count: testResult.teamsCount })}
                                </p>
                            )}
                        </div>
                    )}

                    {(step === 'error' || error) && (
                        <div className="flex items-start gap-2 p-3 bg-red-50 dark:bg-red-900/20 rounded-lg">
                            <XCircle size={16} className="text-red-500 mt-0.5" />
                            <div>
                                <span className="text-sm text-red-700 dark:text-red-300">
                                    {error || t('lan.connectionFailed', 'Connection failed')}
                                </span>
                                {step === 'error' && (
                                    <button
                                        onClick={() => setStep('input')}
                                        className="block text-sm text-red-600 dark:text-red-400 underline mt-1"
                                    >
                                        {t('lan.tryAgain', 'Try again')}
                                    </button>
                                )}
                            </div>
                        </div>
                    )}
                </div>

                {/* Footer */}
                <div className="flex justify-end gap-3 p-4 border-t border-border-subtle">
                    <Button variant="outline" onClick={handleClose} disabled={step === 'testing'}>
                        {t('cancel', 'Cancel')}
                    </Button>

                    {step === 'success' ? (
                        <Button onClick={handleSave}>
                            {t('lan.save', 'Save Connection')}
                        </Button>
                    ) : (
                        <Button
                            onClick={handleTestConnection}
                            disabled={step === 'testing' || !host.trim() || !secretKey.trim()}
                        >
                            {step === 'testing' ? (
                                <>
                                    <Loader2 size={16} className="mr-2 animate-spin" />
                                    {t('lan.testing', 'Testing...')}
                                </>
                            ) : (
                                t('lan.testConnection', 'Test Connection')
                            )}
                        </Button>
                    )}
                </div>
            </div>
        </div>
    );
};

export default ConnectLANDialog;
