// AddSourceDialog - Unified dialog for adding data sources
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, Wifi, X, CheckCircle, XCircle, Loader2, Eye, EyeOff } from 'lucide-react';
import { Button } from '../../ui/button';
import type { DataSource, DataSourceType } from '../sources/types';
import { authAdapter, storeCredential } from '../auth/authAdapter';
import { sourceManager } from '../sources/sourceManager';

interface AddSourceDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess?: (source: DataSource) => void;
  defaultType?: DataSourceType;
}

type Step = 'input' | 'testing' | 'success' | 'error';

export const AddSourceDialog: React.FC<AddSourceDialogProps> = ({
  isOpen,
  onClose,
  onSuccess,
  defaultType = 'cloud',
}) => {
  const { t } = useTranslation('team');

  const [sourceType, setSourceType] = useState<DataSourceType>(defaultType);
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [credential, setCredential] = useState('');
  const [showCredential, setShowCredential] = useState(false);
  const [step, setStep] = useState<Step>('input');
  const [error, setError] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<{
    success: boolean;
    teamsCount?: number;
    userEmail?: string;
    displayName?: string;
  } | null>(null);

  if (!isOpen) return null;

  const resetForm = () => {
    setName('');
    setUrl('');
    setCredential('');
    setShowCredential(false);
    setStep('input');
    setError(null);
    setTestResult(null);
  };

  const handleClose = () => {
    resetForm();
    onClose();
  };

  const handleTest = async () => {
    if (!url.trim() || !credential.trim()) {
      setError(t('addSource.requiredFields', 'URL and credential are required'));
      return;
    }

    setStep('testing');
    setError(null);
    setTestResult(null);

    try {
      const authType = sourceType === 'cloud' ? 'api-key' : 'secret-key';
      const result = await authAdapter.testConnection(url.trim(), authType, credential.trim());

      if (result.success) {
        setTestResult({
          success: true,
          teamsCount: result.teamsCount,
          userEmail: result.userEmail,
          displayName: result.displayName,
        });
        setStep('success');

        // Auto-generate name if not provided
        if (!name.trim()) {
          try {
            const parsed = new URL(url);
            setName(parsed.hostname);
          } catch {
            setName(sourceType === 'cloud' ? 'Cloud Server' : 'LAN Device');
          }
        }
      } else {
        setError(result.error || t('addSource.connectionFailed', 'Connection failed'));
        setStep('error');
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : t('addSource.connectionFailed', 'Connection failed'));
      setStep('error');
    }
  };

  const handleAdd = async () => {
    if (!testResult?.success) {
      setError(t('addSource.testFirst', 'Please test the connection first'));
      return;
    }

    const sourceId = `${sourceType}-${Date.now()}`;
    const authType = sourceType === 'cloud' ? 'api-key' : 'secret-key';

    const newSource: DataSource = {
      id: sourceId,
      type: sourceType,
      name: name.trim() || (sourceType === 'cloud' ? 'Cloud Server' : 'LAN Device'),
      status: 'online',
      connection: {
        url: url.trim().replace(/\/+$/, ''),
        authType,
        credentialRef: sourceId,
      },
      capabilities: {
        canCreate: sourceType === 'cloud',
        canSync: true,
        supportsOffline: false,
        canManageTeams: sourceType === 'cloud',
        canInviteMembers: sourceType === 'cloud',
      },
      teamsCount: testResult.teamsCount,
      userInfo: testResult.userEmail ? {
        email: testResult.userEmail,
        displayName: testResult.displayName,
      } : undefined,
      createdAt: new Date().toISOString(),
    };

    // Store credential
    storeCredential(sourceId, credential.trim());

    // Register source
    sourceManager.registerSource(newSource);

    onSuccess?.(newSource);
    handleClose();
  };

  const isTesting = step === 'testing';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background-default rounded-lg shadow-xl w-full max-w-md mx-4">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border-subtle">
          <div className="flex items-center gap-3">
            <div className={`p-2 rounded-lg ${sourceType === 'cloud' ? 'bg-blue-100 dark:bg-blue-900/30' : 'bg-green-100 dark:bg-green-900/30'}`}>
              {sourceType === 'cloud'
                ? <Cloud size={20} className="text-blue-600 dark:text-blue-400" />
                : <Wifi size={20} className="text-green-600 dark:text-green-400" />
              }
            </div>
            <h2 className="text-lg font-semibold text-text-default">
              {t('addSource.title', 'Add Data Source')}
            </h2>
          </div>
          <button
            onClick={handleClose}
            className="p-1 rounded hover:bg-background-muted text-text-muted hover:text-text-default"
          >
            <X size={20} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4 space-y-4">
          {/* Source Type Selection */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-2">
              {t('addSource.type', 'Type')}
            </label>
            <div className="flex gap-2">
              <button
                className={`flex-1 py-2 px-4 rounded-lg border transition-colors flex items-center justify-center gap-2 ${
                  sourceType === 'cloud'
                    ? 'bg-blue-50 dark:bg-blue-900/20 border-blue-500 text-blue-700 dark:text-blue-300'
                    : 'border-border-subtle hover:border-border-default text-text-default'
                }`}
                onClick={() => setSourceType('cloud')}
                disabled={isTesting}
              >
                <Cloud size={16} />
                {t('addSource.cloud', 'Cloud')}
              </button>
              <button
                className={`flex-1 py-2 px-4 rounded-lg border transition-colors flex items-center justify-center gap-2 ${
                  sourceType === 'lan'
                    ? 'bg-green-50 dark:bg-green-900/20 border-green-500 text-green-700 dark:text-green-300'
                    : 'border-border-subtle hover:border-border-default text-text-default'
                }`}
                onClick={() => setSourceType('lan')}
                disabled={isTesting}
              >
                <Wifi size={16} />
                {t('addSource.lan', 'LAN')}
              </button>
            </div>
          </div>

          {/* Name */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-1">
              {t('addSource.name', 'Display Name')}
            </label>
            <input
              type="text"
              className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 disabled:opacity-50"
              placeholder={sourceType === 'cloud' ? t('addSource.namePlaceholderCloud', 'My Cloud Server') : t('addSource.namePlaceholderLan', 'Office PC')}
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={isTesting}
            />
          </div>

          {/* URL */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-1">
              {sourceType === 'cloud' ? t('addSource.serverUrl', 'Server URL') : t('addSource.deviceAddress', 'Device Address')} *
            </label>
            <input
              type="url"
              className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 disabled:opacity-50"
              placeholder={
                sourceType === 'cloud'
                  ? 'https://team.example.com'
                  : 'http://192.168.1.100:7778'
              }
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              disabled={isTesting}
            />
          </div>

          {/* Credential */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-1">
              {sourceType === 'cloud' ? t('addSource.apiKey', 'API Key') : t('addSource.secretKey', 'Secret Key')} *
            </label>
            <div className="relative">
              <input
                type={showCredential ? 'text' : 'password'}
                className="w-full px-3 py-2 pr-10 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-teal-500 disabled:opacity-50"
                placeholder={t('addSource.credentialPlaceholder', 'Enter key...')}
                value={credential}
                onChange={(e) => setCredential(e.target.value)}
                disabled={isTesting}
              />
              <button
                type="button"
                onClick={() => setShowCredential(!showCredential)}
                className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-text-muted hover:text-text-default"
              >
                {showCredential ? <EyeOff size={16} /> : <Eye size={16} />}
              </button>
            </div>
          </div>

          {/* Status messages */}
          {isTesting && (
            <div className="flex items-center gap-2 p-3 bg-blue-50 dark:bg-blue-900/20 rounded-lg">
              <Loader2 size={16} className="text-blue-500 animate-spin" />
              <span className="text-sm text-blue-700 dark:text-blue-300">
                {t('addSource.testing', 'Testing connection...')}
              </span>
            </div>
          )}

          {step === 'success' && testResult?.success && (
            <div className="p-3 bg-green-50 dark:bg-green-900/20 rounded-lg">
              <div className="flex items-center gap-2 mb-1">
                <CheckCircle size={16} className="text-green-500" />
                <span className="text-sm font-medium text-green-700 dark:text-green-300">
                  {t('addSource.connectionSuccess', 'Connected successfully!')}
                </span>
              </div>
              {testResult.teamsCount !== undefined && (
                <p className="text-sm text-green-600 dark:text-green-400 ml-6">
                  {t('addSource.teamsFound', 'Found {{count}} teams', { count: testResult.teamsCount })}
                </p>
              )}
              {testResult.userEmail && (
                <p className="text-xs text-green-600/80 dark:text-green-400/80 ml-6">
                  {testResult.displayName || testResult.userEmail}
                </p>
              )}
            </div>
          )}

          {(step === 'error' || error) && (
            <div className="flex items-start gap-2 p-3 bg-red-50 dark:bg-red-900/20 rounded-lg">
              <XCircle size={16} className="text-red-500 mt-0.5" />
              <div>
                <span className="text-sm text-red-700 dark:text-red-300">
                  {error || t('addSource.connectionFailed', 'Connection failed')}
                </span>
                {step === 'error' && (
                  <button
                    onClick={() => setStep('input')}
                    className="block text-sm text-red-600 dark:text-red-400 underline mt-1"
                  >
                    {t('addSource.tryAgain', 'Try again')}
                  </button>
                )}
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-3 p-4 border-t border-border-subtle">
          <Button variant="outline" onClick={handleClose} disabled={isTesting}>
            {t('cancel', 'Cancel')}
          </Button>

          {step === 'success' ? (
            <Button onClick={handleAdd}>
              {t('addSource.add', 'Add Source')}
            </Button>
          ) : (
            <Button
              onClick={handleTest}
              disabled={isTesting || !url.trim() || !credential.trim()}
            >
              {isTesting ? (
                <>
                  <Loader2 size={16} className="mr-2 animate-spin" />
                  {t('addSource.testing', 'Testing...')}
                </>
              ) : (
                t('addSource.testConnection', 'Test Connection')
              )}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
};
