import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Input } from '../../ui/input';
import { Check, Lock, Loader2, AlertCircle, Share2 } from 'lucide-react';
import { Switch } from '../../ui/switch';
import { Button } from '../../ui/button';
import { SettingsCard } from '../common';
import { getConfigCompat } from '../../../utils/envCompat';

export default function SessionSharingSection() {
  const { t } = useTranslation('settings');
  const envBaseUrlShare = getConfigCompat('BASE_URL_SHARE');
  console.log('envBaseUrlShare', envBaseUrlShare);

  // If env is set, force sharing enabled and set the baseUrl accordingly.
  const [sessionSharingConfig, setSessionSharingConfig] = useState({
    enabled: envBaseUrlShare ? true : false,
    baseUrl: typeof envBaseUrlShare === 'string' ? envBaseUrlShare : '',
  });
  const [urlError, setUrlError] = useState('');
  const [testResult, setTestResult] = useState<{
    status: 'success' | 'error' | 'testing' | null;
    message: string;
  }>({ status: null, message: '' });

  // isUrlConfigured is true if the user has configured a baseUrl and it is valid.
  const isUrlConfigured =
    !envBaseUrlShare &&
    sessionSharingConfig.enabled &&
    isValidUrl(String(sessionSharingConfig.baseUrl));

  // Only load saved config from localStorage if the env variable is not provided.
  useEffect(() => {
    if (envBaseUrlShare) {
      // If env variable is set, save the forced configuration to localStorage
      const forcedConfig = {
        enabled: true,
        baseUrl: typeof envBaseUrlShare === 'string' ? envBaseUrlShare : '',
      };
      localStorage.setItem('session_sharing_config', JSON.stringify(forcedConfig));
    } else {
      const savedSessionConfig = localStorage.getItem('session_sharing_config');
      if (savedSessionConfig) {
        try {
          const config = JSON.parse(savedSessionConfig);
          setSessionSharingConfig(config);
        } catch (error) {
          console.error('Error parsing session sharing config:', error);
        }
      }
    }
  }, [envBaseUrlShare]);

  // Helper to check if the user's input is a valid URL
  function isValidUrl(value: string): boolean {
    if (!value) return false;
    try {
      new URL(value);
      return true;
    } catch {
      return false;
    }
  }

  // Toggle sharing (only allowed when env is not set).
  const toggleSharing = () => {
    if (envBaseUrlShare) {
      return; // Do nothing if the environment variable forces sharing.
    }
    setSessionSharingConfig((prev) => {
      const updated = { ...prev, enabled: !prev.enabled };
      localStorage.setItem('session_sharing_config', JSON.stringify(updated));
      return updated;
    });
  };

  // Handle changes to the base URL field
  const handleBaseUrlChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newBaseUrl = e.target.value;
    setSessionSharingConfig((prev) => ({
      ...prev,
      baseUrl: newBaseUrl,
    }));

    // Clear previous test results when URL changes
    setTestResult({ status: null, message: '' });

    if (isValidUrl(newBaseUrl)) {
      setUrlError('');
      const updated = { ...sessionSharingConfig, baseUrl: newBaseUrl };
      localStorage.setItem('session_sharing_config', JSON.stringify(updated));
    } else {
      setUrlError(t('sessionSharing.invalidUrl'));
    }
  };

  // Test connection to the configured URL
  const testConnection = async () => {
    const baseUrl = sessionSharingConfig.baseUrl;
    if (!baseUrl) return;

    setTestResult({ status: 'testing', message: t('sessionSharing.testingConnection') });

    try {
      // Create an AbortController for timeout
      const controller = new AbortController();
      const timeoutId = window.setTimeout(() => controller.abort(), 10000); // 10 second timeout

      const response = await fetch(baseUrl, {
        method: 'GET',
        headers: {
          Accept: 'application/json, text/plain, */*',
        },
        signal: controller.signal,
      });

      window.clearTimeout(timeoutId);

      // Consider any response (even 404) as a successful connection
      // since it means we can reach the server
      if (response.status < 500) {
        setTestResult({
          status: 'success',
          message: t('sessionSharing.connectionSuccess'),
        });
      } else {
        setTestResult({
          status: 'error',
          message: t('sessionSharing.serverError', { status: response.status }),
        });
      }
    } catch (error) {
      console.error('Connection test failed:', error);
      let errorMessage = t('sessionSharing.connectionFailed') + ' ';

      if (error instanceof TypeError && error.message.includes('fetch')) {
        errorMessage += t('sessionSharing.unableToReach');
      } else if (error instanceof Error) {
        if (error.name === 'AbortError') {
          errorMessage += t('sessionSharing.connectionTimeout');
        } else {
          errorMessage += error.message;
        }
      } else {
        errorMessage += t('sessionSharing.unknownError');
      }

      setTestResult({
        status: 'error',
        message: errorMessage,
      });
    }
  };

  return (
    <SettingsCard
      icon={<Share2 className="h-5 w-5" />}
      title={t('sessionSharing.title')}
      description={
        (envBaseUrlShare as string)
          ? t('sessionSharing.descriptionConfigured')
          : t('sessionSharing.descriptionNotConfigured')
      }
    >
      {/* Toggle for enabling session sharing */}
      <div className="flex items-center justify-between py-2 px-2 hover:bg-background-muted rounded-lg transition-colors">
        <div className="flex-1">
          <h4 className="text-sm font-medium text-text-default leading-5">
            {(envBaseUrlShare as string)
              ? t('sessionSharing.alreadyConfigured')
              : t('sessionSharing.enableSharing')}
          </h4>
        </div>
        <div className="flex-shrink-0">
          {envBaseUrlShare ? (
            <Lock className="w-5 h-5 text-text-muted" />
          ) : (
            <Switch
              checked={sessionSharingConfig.enabled}
              disabled={!!envBaseUrlShare}
              onCheckedChange={toggleSharing}
              variant="mono"
            />
          )}
        </div>
      </div>

      {/* Base URL field (only visible if enabled) */}
      {sessionSharingConfig.enabled && (
        <div className="space-y-3 px-2">
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <label htmlFor="session-sharing-url" className="text-sm font-medium text-text-default">
                {t('sessionSharing.baseUrl')}
              </label>
              {isUrlConfigured && <Check className="w-4 h-4 text-green-500" />}
            </div>
            <Input
              id="session-sharing-url"
              type="url"
              placeholder="https://example.com/api"
              value={sessionSharingConfig.baseUrl}
              disabled={!!envBaseUrlShare}
              {...(envBaseUrlShare ? {} : { onChange: handleBaseUrlChange })}
            />
            {urlError && <p className="text-red-500 text-xs">{urlError}</p>}
          </div>

          {(isUrlConfigured || (envBaseUrlShare as string)) && (
            <div className="space-y-2">
              <Button
                variant="outline"
                size="sm"
                onClick={testConnection}
                disabled={testResult.status === 'testing'}
                className="flex items-center gap-2"
              >
                {testResult.status === 'testing' ? (
                  <>
                    <Loader2 className="w-4 h-4 animate-spin" />
                    {t('sessionSharing.testing')}
                  </>
                ) : (
                  t('sessionSharing.testConnection')
                )}
              </Button>

              {/* Test Results */}
              {testResult.status && testResult.status !== 'testing' && (
                <div
                  className={`flex items-start gap-2 p-3 rounded-lg text-sm ${
                    testResult.status === 'success'
                      ? 'bg-green-50 dark:bg-green-900/20 text-green-800 dark:text-green-200 border border-green-200 dark:border-green-800'
                      : 'bg-red-50 dark:bg-red-900/20 text-red-800 dark:text-red-200 border border-red-200 dark:border-red-800'
                  }`}
                >
                  {testResult.status === 'success' ? (
                    <Check className="w-4 h-4 mt-0.5 flex-shrink-0" />
                  ) : (
                    <AlertCircle className="w-4 h-4 mt-0.5 flex-shrink-0" />
                  )}
                  <span>{testResult.message}</span>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </SettingsCard>
  );
}
