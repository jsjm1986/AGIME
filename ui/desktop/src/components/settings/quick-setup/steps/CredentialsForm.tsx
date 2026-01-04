import React, { memo, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Key, Globe, ExternalLink, Loader2, CheckCircle, XCircle, Lock, Unlock } from 'lucide-react';
import { cn } from '../../../../utils';
import type { ProviderPreset } from '../data/providerPresets';

export interface CredentialsData {
  apiKey: string;
  baseUrl: string;
  useCustomUrl: boolean;
  engine: 'openai' | 'anthropic' | 'ollama';
}

interface CredentialsFormProps {
  provider: ProviderPreset;
  credentials: CredentialsData;
  onChange: (credentials: CredentialsData) => void;
  onValidate: () => Promise<boolean>;
  validationState: 'idle' | 'validating' | 'success' | 'error';
  validationMessage?: string;
}

export const CredentialsForm = memo(function CredentialsForm({
  provider,
  credentials,
  onChange,
  onValidate,
  validationState,
  validationMessage,
}: CredentialsFormProps) {
  const { t } = useTranslation('settings');
  const [showApiKey, setShowApiKey] = useState(false);
  const isCustomProvider = provider.id === 'custom';

  const handleApiKeyChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    onChange({ ...credentials, apiKey: e.target.value });
  }, [credentials, onChange]);

  const handleBaseUrlChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    onChange({ ...credentials, baseUrl: e.target.value });
  }, [credentials, onChange]);

  const handleUseCustomUrlChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const useCustom = e.target.checked;
    onChange({
      ...credentials,
      useCustomUrl: useCustom,
      baseUrl: useCustom ? credentials.baseUrl : provider.baseUrl,
    });
  }, [credentials, onChange, provider.baseUrl]);

  const handleEngineChange = useCallback((engine: 'openai' | 'anthropic') => {
    onChange({ ...credentials, engine });
  }, [credentials, onChange]);

  const toggleShowApiKey = useCallback(() => {
    setShowApiKey(prev => !prev);
  }, []);

  return (
    <div className="space-y-5">
      {/* Provider info header */}
      <div className="p-4 bg-gradient-to-br from-block-teal/10 to-block-orange/5 rounded-xl border border-block-teal/20">
        <h3 className="text-base font-semibold text-text-default">{provider.displayName}</h3>
        <p className="text-sm text-text-muted mt-1">{provider.description}</p>
      </div>

      {/* API Key input */}
      <div className="space-y-2">
        <label className="flex items-center gap-2 text-sm font-semibold text-text-default">
          <Key className="w-4 h-4 text-block-teal" />
          API Key
          <span className="text-red-500">*</span>
        </label>
        <div className="relative">
          <input
            type={showApiKey ? 'text' : 'password'}
            value={credentials.apiKey}
            onChange={handleApiKeyChange}
            placeholder={t('quickSetup.credentials.apiKeyPlaceholder', { keyName: provider.apiKeyEnv.replace(/_/g, ' ') })}
            className={cn(
              'w-full px-4 py-3 pr-12 rounded-xl border text-sm',
              'bg-background-default text-text-default',
              'placeholder-text-muted',
              'focus:outline-none focus:ring-2 focus:ring-block-teal/50 focus:border-block-teal',
              'transition-all duration-200',
              validationState === 'error'
                ? 'border-red-400 focus:ring-red-400/50'
                : 'border-border-default hover:border-block-teal/50'
            )}
          />
          <button
            type="button"
            onClick={toggleShowApiKey}
            className="absolute right-3 top-1/2 -translate-y-1/2 p-1.5 rounded-lg text-text-muted hover:text-block-teal hover:bg-block-teal/10 transition-colors"
          >
            {showApiKey ? <Unlock className="w-4 h-4" /> : <Lock className="w-4 h-4" />}
          </button>
        </div>
        {provider.apiKeyHelpUrl && (
          <a
            href={provider.apiKeyHelpUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1.5 text-sm text-block-teal hover:text-block-teal/80 hover:underline transition-colors"
          >
            <ExternalLink className="w-3.5 h-3.5" />
            {t('quickSetup.credentials.getApiKey')}
          </a>
        )}
      </div>

      {/* API URL input */}
      <div className="space-y-2">
        <label className="flex items-center gap-2 text-sm font-semibold text-text-default">
          <Globe className="w-4 h-4 text-block-teal" />
          {t('quickSetup.credentials.apiAddress')}
          {isCustomProvider && <span className="text-red-500">*</span>}
          {!isCustomProvider && (
            <span className="text-xs text-text-muted font-normal px-2 py-0.5 bg-background-muted rounded-full">{t('quickSetup.credentials.prefilled')}</span>
          )}
        </label>
        <div className="relative">
          <input
            type="text"
            value={credentials.baseUrl}
            onChange={handleBaseUrlChange}
            disabled={!isCustomProvider && !credentials.useCustomUrl}
            placeholder="https://api.example.com/v1"
            className={cn(
              'w-full px-4 py-3 rounded-xl border text-sm',
              'bg-background-default text-text-default',
              'placeholder-text-muted',
              'focus:outline-none focus:ring-2 focus:ring-block-teal/50 focus:border-block-teal',
              'transition-all duration-200',
              'border-border-default hover:border-block-teal/50',
              (!isCustomProvider && !credentials.useCustomUrl) && 'opacity-60 cursor-not-allowed bg-background-muted'
            )}
          />
          {!isCustomProvider && !credentials.useCustomUrl && (
            <Lock className="absolute right-3 top-1/2 -translate-y-1/2 w-4 h-4 text-text-muted" />
          )}
        </div>
        {!isCustomProvider && (
          <label className="flex items-center gap-2.5 text-sm text-text-muted cursor-pointer group">
            <input
              type="checkbox"
              checked={credentials.useCustomUrl}
              onChange={handleUseCustomUrlChange}
              className="w-4 h-4 rounded border-border-default text-block-teal focus:ring-block-teal/50 cursor-pointer"
            />
            <span className="group-hover:text-text-default transition-colors">{t('quickSetup.credentials.useCustomAddress')}</span>
          </label>
        )}
      </div>

      {/* Engine selection for custom provider */}
      {isCustomProvider && (
        <div className="space-y-2">
          <label className="block text-sm font-semibold text-text-default">
            {t('quickSetup.credentials.apiProtocol')}
          </label>
          <div className="flex gap-3">
            <label className={cn(
              'flex-1 flex items-center justify-center gap-2 px-4 py-3 rounded-xl border-2 cursor-pointer transition-all duration-200',
              credentials.engine === 'openai'
                ? 'border-block-teal bg-block-teal/10 shadow-sm'
                : 'border-border-default hover:border-block-teal/50 hover:bg-background-muted'
            )}>
              <input
                type="radio"
                name="engine"
                value="openai"
                checked={credentials.engine === 'openai'}
                onChange={() => handleEngineChange('openai')}
                className="sr-only"
              />
              <span className={cn(
                'text-sm font-medium',
                credentials.engine === 'openai' ? 'text-block-teal' : 'text-text-default'
              )}>
                {t('quickSetup.credentials.openaiCompatible')}
              </span>
              <span className="text-xs px-2 py-0.5 rounded-full bg-block-teal/20 text-block-teal">{t('quickSetup.credentials.recommended')}</span>
            </label>
            <label className={cn(
              'flex-1 flex items-center justify-center gap-2 px-4 py-3 rounded-xl border-2 cursor-pointer transition-all duration-200',
              credentials.engine === 'anthropic'
                ? 'border-block-teal bg-block-teal/10 shadow-sm'
                : 'border-border-default hover:border-block-teal/50 hover:bg-background-muted'
            )}>
              <input
                type="radio"
                name="engine"
                value="anthropic"
                checked={credentials.engine === 'anthropic'}
                onChange={() => handleEngineChange('anthropic')}
                className="sr-only"
              />
              <span className={cn(
                'text-sm font-medium',
                credentials.engine === 'anthropic' ? 'text-block-teal' : 'text-text-default'
              )}>
                {t('quickSetup.credentials.anthropicCompatible')}
              </span>
            </label>
          </div>
        </div>
      )}

      {/* Validation button and status */}
      <div className="pt-3">
        <button
          type="button"
          onClick={onValidate}
          disabled={!credentials.apiKey || validationState === 'validating'}
          className={cn(
            'w-full px-4 py-3 rounded-xl font-semibold text-sm transition-all duration-200',
            'flex items-center justify-center gap-2',
            'focus:outline-none focus-visible:ring-2 focus-visible:ring-offset-2',
            !credentials.apiKey || validationState === 'validating'
              ? 'bg-background-muted text-text-muted cursor-not-allowed border border-border-default'
              : 'bg-gradient-to-r from-block-teal to-block-teal/80 text-white hover:shadow-lg hover:shadow-block-teal/25 focus-visible:ring-block-teal'
          )}
        >
          {validationState === 'validating' ? (
            <>
              <Loader2 className="w-4 h-4 animate-spin" />
              {t('quickSetup.credentials.validating')}
            </>
          ) : (
            t('quickSetup.credentials.validate')
          )}
        </button>

        {/* Validation result */}
        {validationState !== 'idle' && validationState !== 'validating' && (
          <div className={cn(
            'mt-4 p-4 rounded-xl flex items-start gap-3 text-sm',
            validationState === 'success'
              ? 'bg-green-500/10 border border-green-500/20'
              : 'bg-red-500/10 border border-red-500/20'
          )}>
            {validationState === 'success' ? (
              <CheckCircle className="w-5 h-5 text-green-500 flex-shrink-0" />
            ) : (
              <XCircle className="w-5 h-5 text-red-500 flex-shrink-0" />
            )}
            <span className={validationState === 'success' ? 'text-green-600 dark:text-green-400' : 'text-red-600 dark:text-red-400'}>
              {validationMessage || (validationState === 'success' ? t('quickSetup.credentials.validationSuccess') : t('quickSetup.credentials.validationFailed'))}
            </span>
          </div>
        )}
      </div>
    </div>
  );
});

export default CredentialsForm;
