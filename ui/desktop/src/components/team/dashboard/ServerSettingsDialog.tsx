// ServerSettingsDialog - Dialog for editing server settings
import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Cloud, X, Eye, EyeOff, Loader2, CheckCircle, XCircle } from 'lucide-react';
import { Button } from '../../ui/button';
import type { DataSource } from '../sources/types';
import { authAdapter, storeCredential, getCredential } from '../auth/authAdapter';
import { sourceManager } from '../sources/sourceManager';

interface ServerSettingsDialogProps {
  isOpen: boolean;
  source: DataSource | null;
  onClose: () => void;
  onSave?: (source: DataSource) => void;
}

type Step = 'input' | 'testing' | 'success' | 'error';

export const ServerSettingsDialog: React.FC<ServerSettingsDialogProps> = ({
  isOpen,
  source,
  onClose,
  onSave,
}) => {
  const { t } = useTranslation('team');

  const [name, setName] = useState('');
  const [credential, setCredential] = useState('');
  const [showCredential, setShowCredential] = useState(false);
  const [step, setStep] = useState<Step>('input');
  const [error, setError] = useState<string | null>(null);
  const [hasChanges, setHasChanges] = useState(false);

  // Initialize form when source changes
  useEffect(() => {
    if (source) {
      setName(source.name);
      // Get stored credential
      const storedCred = getCredential(source.connection.credentialRef);
      setCredential(storedCred || '');
      setStep('input');
      setError(null);
      setHasChanges(false);
    }
  }, [source]);

  if (!isOpen || !source) return null;

  const handleClose = () => {
    setStep('input');
    setError(null);
    setShowCredential(false);
    onClose();
  };

  const handleNameChange = (value: string) => {
    setName(value);
    setHasChanges(true);
  };

  const handleCredentialChange = (value: string) => {
    setCredential(value);
    setHasChanges(true);
  };

  const handleTestConnection = async () => {
    if (!credential.trim()) {
      setError(t('serverSettings.credentialRequired', 'API Key is required'));
      return;
    }

    setStep('testing');
    setError(null);

    try {
      const result = await authAdapter.testConnection(
        source.connection.url,
        source.connection.authType,
        credential.trim()
      );

      if (result.success) {
        setStep('success');
      } else {
        setError(result.error || t('addSource.connectionFailed', 'Connection failed'));
        setStep('error');
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : t('addSource.connectionFailed', 'Connection failed'));
      setStep('error');
    }
  };

  const handleSave = async () => {
    // Update credential if changed
    if (credential.trim()) {
      storeCredential(source.connection.credentialRef, credential.trim());
    }

    // Update in source manager
    const updatedSource = sourceManager.updateSource(source.id, {
      name: name.trim() || source.name,
    });

    onSave?.(updatedSource || source);
    handleClose();
  };

  const isTesting = step === 'testing';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-background-default rounded-xl shadow-xl w-full max-w-md mx-4">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-border-subtle">
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-blue-100 dark:bg-blue-900/30">
              <Cloud size={20} className="text-blue-600 dark:text-blue-400" />
            </div>
            <h2 className="text-lg font-semibold text-text-default">
              {t('serverSettings.title', 'Server Settings')}
            </h2>
          </div>
          <button
            onClick={handleClose}
            className="p-1.5 rounded-lg hover:bg-background-muted text-text-muted hover:text-text-default"
          >
            <X size={20} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4 space-y-4">
          {/* Server URL (read-only) */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-1">
              {t('addSource.serverUrl', 'Server URL')}
            </label>
            <div className="px-3 py-2 border border-border-subtle rounded-lg bg-background-muted text-text-muted text-sm">
              {source.connection.url}
            </div>
          </div>

          {/* Display Name */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-1">
              {t('addSource.name', 'Display Name')}
            </label>
            <input
              type="text"
              className="w-full px-3 py-2 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
              placeholder={t('addSource.namePlaceholderCloud', 'My Cloud Server')}
              value={name}
              onChange={(e) => handleNameChange(e.target.value)}
              disabled={isTesting}
            />
          </div>

          {/* API Key */}
          <div>
            <label className="block text-sm font-medium text-text-default mb-1">
              {t('addSource.apiKey', 'API Key')}
            </label>
            <div className="relative">
              <input
                type={showCredential ? 'text' : 'password'}
                className="w-full px-3 py-2 pr-10 border border-border-subtle rounded-lg bg-background-default text-text-default focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-50"
                placeholder={t('serverSettings.apiKeyPlaceholder', 'Enter new API Key to update...')}
                value={credential}
                onChange={(e) => handleCredentialChange(e.target.value)}
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
            <p className="mt-1 text-xs text-text-muted">
              {t('serverSettings.apiKeyHint', 'Leave unchanged to keep current key')}
            </p>
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

          {step === 'success' && (
            <div className="flex items-center gap-2 p-3 bg-green-50 dark:bg-green-900/20 rounded-lg">
              <CheckCircle size={16} className="text-green-500" />
              <span className="text-sm font-medium text-green-700 dark:text-green-300">
                {t('addSource.connectionSuccess', 'Connected successfully!')}
              </span>
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
        <div className="flex justify-between gap-3 p-4 border-t border-border-subtle">
          <Button
            variant="outline"
            onClick={handleTestConnection}
            disabled={isTesting || !credential.trim()}
          >
            {isTesting ? (
              <>
                <Loader2 size={14} className="mr-2 animate-spin" />
                {t('addSource.testing', 'Testing...')}
              </>
            ) : (
              t('addSource.testConnection', 'Test Connection')
            )}
          </Button>

          <div className="flex gap-2">
            <Button variant="outline" onClick={handleClose} disabled={isTesting}>
              {t('cancel', 'Cancel')}
            </Button>
            <Button
              onClick={handleSave}
              disabled={isTesting || !hasChanges}
              className="bg-blue-600 hover:bg-blue-700"
            >
              {t('serverSettings.save', 'Save')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
};
