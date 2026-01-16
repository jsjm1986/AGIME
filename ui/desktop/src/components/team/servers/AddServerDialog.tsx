import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, CheckCircle, XCircle, Loader2, Eye, EyeOff } from 'lucide-react';
import { Button } from '../../ui/button';
import { CloudServer } from '../types';
import { addServer, testServerConnection } from './serverStore';

interface AddServerDialogProps {
    open: boolean;
    onClose: () => void;
    onSuccess: (server: CloudServer) => void;
}

type Step = 'input' | 'testing' | 'success' | 'error';

const AddServerDialog: React.FC<AddServerDialogProps> = ({
    open,
    onClose,
    onSuccess,
}) => {
    const { t } = useTranslation('team');

    const [step, setStep] = useState<Step>('input');
    const [serverUrl, setServerUrl] = useState('');
    const [apiKey, setApiKey] = useState('');
    const [serverName, setServerName] = useState('');
    const [showApiKey, setShowApiKey] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [testResult, setTestResult] = useState<{
        userEmail?: string;
        displayName?: string;
    } | null>(null);

    const resetForm = () => {
        setStep('input');
        setServerUrl('');
        setApiKey('');
        setServerName('');
        setError(null);
        setTestResult(null);
        setShowApiKey(false);
    };

    const handleClose = () => {
        resetForm();
        onClose();
    };

    const handleTestConnection = async () => {
        if (!serverUrl.trim() || !apiKey.trim()) {
            setError(t('server.requiredFields', 'Server URL and API Key are required'));
            return;
        }

        setStep('testing');
        setError(null);

        const result = await testServerConnection(serverUrl.trim(), apiKey.trim());

        if (result.success) {
            setTestResult({
                userEmail: result.userEmail,
                displayName: result.displayName,
            });
            setStep('success');

            // Auto-generate server name if not provided
            if (!serverName.trim()) {
                try {
                    const url = new URL(serverUrl);
                    setServerName(url.hostname);
                } catch {
                    setServerName('Cloud Server');
                }
            }
        } else {
            setError(result.error || t('server.connectionFailed', 'Connection failed'));
            setStep('error');
        }
    };

    const handleSave = () => {
        try {
            const server = addServer({
                name: serverName.trim() || 'Cloud Server',
                url: serverUrl.trim(),
                apiKey: apiKey.trim(),
            });

            // Update with test result info
            if (testResult) {
                server.userEmail = testResult.userEmail;
                server.displayName = testResult.displayName;
                server.status = 'online';
            }

            onSuccess(server);
            handleClose();
        } catch (e) {
            setError(e instanceof Error ? e.message : 'Failed to save server');
            setStep('error');
        }
    };

    if (!open) return null;

    return (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
            <div className="bg-background-default rounded-lg shadow-xl w-full max-w-md mx-4">
                {/* Header */}
                <div className="flex items-center gap-3 p-4 border-b border-border-subtle">
                    <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/30">
                        <Cloud size={20} className="text-blue-600 dark:text-blue-400" />
                    </div>
                    <h2 className="text-lg font-semibold text-text-default">
                        {t('server.addTitle', 'Add Cloud Server')}
                    </h2>
                </div>

                {/* Content */}
                <div className="p-4 space-y-4">
                    {/* Server URL */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('server.url', 'Server URL')} *
                        </label>
                        <input
                            type="url"
                            value={serverUrl}
                            onChange={(e) => setServerUrl(e.target.value)}
                            placeholder="https://team.company.com"
                            disabled={step === 'testing'}
                            className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 disabled:opacity-50"
                        />
                    </div>

                    {/* API Key */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('server.apiKey', 'API Key')} *
                        </label>
                        <div className="relative">
                            <input
                                type={showApiKey ? 'text' : 'password'}
                                value={apiKey}
                                onChange={(e) => setApiKey(e.target.value)}
                                placeholder="agime_xxx_..."
                                disabled={step === 'testing'}
                                className="w-full px-3 py-2 pr-10 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 disabled:opacity-50"
                            />
                            <button
                                type="button"
                                onClick={() => setShowApiKey(!showApiKey)}
                                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-text-muted hover:text-text-default"
                            >
                                {showApiKey ? <EyeOff size={16} /> : <Eye size={16} />}
                            </button>
                        </div>
                        <p className="text-xs text-text-muted mt-1">
                            {t('server.apiKeyHint', 'Get your API Key from the server admin or register on the server')}
                        </p>
                    </div>

                    {/* Server Name */}
                    <div>
                        <label className="block text-sm font-medium text-text-default mb-1">
                            {t('server.name', 'Display Name')}
                        </label>
                        <input
                            type="text"
                            value={serverName}
                            onChange={(e) => setServerName(e.target.value)}
                            placeholder={t('server.namePlaceholder', 'e.g., Company Team Server')}
                            disabled={step === 'testing'}
                            className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 disabled:opacity-50"
                        />
                    </div>

                    {/* Status messages */}
                    {step === 'testing' && (
                        <div className="flex items-center gap-2 p-3 bg-blue-50 dark:bg-blue-900/20 rounded-lg">
                            <Loader2 size={16} className="text-blue-500 animate-spin" />
                            <span className="text-sm text-blue-700 dark:text-blue-300">
                                {t('server.testing', 'Testing connection...')}
                            </span>
                        </div>
                    )}

                    {step === 'success' && testResult && (
                        <div className="p-3 bg-green-50 dark:bg-green-900/20 rounded-lg">
                            <div className="flex items-center gap-2 mb-2">
                                <CheckCircle size={16} className="text-green-500" />
                                <span className="text-sm font-medium text-green-700 dark:text-green-300">
                                    {t('server.connected', 'Connected successfully!')}
                                </span>
                            </div>
                            {testResult.displayName && (
                                <p className="text-sm text-green-600 dark:text-green-400">
                                    {t('server.loggedInAs', 'Logged in as: {{name}}', { name: testResult.displayName })}
                                </p>
                            )}
                            {testResult.userEmail && (
                                <p className="text-xs text-green-600/80 dark:text-green-400/80">
                                    {testResult.userEmail}
                                </p>
                            )}
                        </div>
                    )}

                    {(step === 'error' || error) && (
                        <div className="flex items-start gap-2 p-3 bg-red-50 dark:bg-red-900/20 rounded-lg">
                            <XCircle size={16} className="text-red-500 mt-0.5" />
                            <div>
                                <span className="text-sm text-red-700 dark:text-red-300">
                                    {error || t('server.connectionFailed', 'Connection failed')}
                                </span>
                                {step === 'error' && (
                                    <button
                                        onClick={() => setStep('input')}
                                        className="block text-sm text-red-600 dark:text-red-400 underline mt-1"
                                    >
                                        {t('server.tryAgain', 'Try again')}
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
                            {t('server.save', 'Save Server')}
                        </Button>
                    ) : (
                        <Button
                            onClick={handleTestConnection}
                            disabled={step === 'testing' || !serverUrl.trim() || !apiKey.trim()}
                        >
                            {step === 'testing' ? (
                                <>
                                    <Loader2 size={16} className="mr-2 animate-spin" />
                                    {t('server.testing', 'Testing...')}
                                </>
                            ) : (
                                t('server.testConnection', 'Test Connection')
                            )}
                        </Button>
                    )}
                </div>
            </div>
        </div>
    );
};

export default AddServerDialog;
