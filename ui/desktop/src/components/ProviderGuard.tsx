import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useConfig } from './ConfigContext';
import { useModelAndProvider } from './ModelAndProviderContext';
import WelcomeAgimeLogo from './WelcomeAgimeLogo';
import { toastService, toastSuccess } from '../toasts';
import { OllamaSetup } from './OllamaSetup';
import TelemetrySettings from './settings/app/TelemetrySettings';
import { buildAgimeKey, buildGooseKey } from '../utils/envCompat';
import { QuickSetupModal, type QuickSetupConfig } from './settings/quick-setup';
import { Sparkles } from 'lucide-react';

import { Goose } from './icons';

interface ProviderGuardProps {
  didSelectProvider: boolean;
  children: React.ReactNode;
}

export default function ProviderGuard({ didSelectProvider, children }: ProviderGuardProps) {
  const { t } = useTranslation('welcome');
  const { read } = useConfig();
  const { refreshCurrentModelAndProvider } = useModelAndProvider();
  const navigate = useNavigate();
  const [isChecking, setIsChecking] = useState(true);
  const [hasProvider, setHasProvider] = useState(false);
  const [showFirstTimeSetup, setShowFirstTimeSetup] = useState(false);
  const [showOllamaSetup, setShowOllamaSetup] = useState(false);
  const [showQuickSetup, setShowQuickSetup] = useState(false);

  const handleOllamaComplete = () => {
    setShowOllamaSetup(false);
    setShowFirstTimeSetup(false);
    setHasProvider(true);
    navigate('/', { replace: true });
  };

  const handleOllamaCancel = () => {
    setShowOllamaSetup(false);
  };

  const handleQuickSetupComplete = useCallback(async (config: QuickSetupConfig) => {
    setShowQuickSetup(false);
    setShowFirstTimeSetup(false);
    setHasProvider(true);
    // Refresh the global model/provider context to sync across the app
    await refreshCurrentModelAndProvider();
    toastSuccess({
      title: t('quickSetupWizard.configSuccess'),
      msg: t('quickSetupWizard.configSuccessMsg', { provider: config.provider.displayName, model: config.modelName }),
    });
    navigate('/', { replace: true });
  }, [navigate, refreshCurrentModelAndProvider, t]);

  useEffect(() => {
    const checkProvider = async () => {
      try {
        // Try AGIME_PROVIDER first, fallback to GOOSE_PROVIDER
        let provider = ((await read(buildAgimeKey('PROVIDER'), false)) as string) || '';
        if (!provider || provider.trim() === '') {
          provider = ((await read(buildGooseKey('PROVIDER'), false)) as string) || '';
        }
        const hasConfiguredProvider = provider.trim() !== '';

        if (hasConfiguredProvider || didSelectProvider) {
          setHasProvider(true);
          setShowFirstTimeSetup(false);
        } else {
          setHasProvider(false);
          setShowFirstTimeSetup(true);
        }
      } catch (error) {
        console.error('Error checking provider:', error);
        toastService.error({
          title: t('configError'),
          msg: t('configErrorMsg'),
          traceback: error instanceof Error ? error.stack || '' : '',
        });
        setHasProvider(false);
        setShowFirstTimeSetup(true);
      } finally {
        setIsChecking(false);
      }
    };

    checkProvider();
  }, [read, didSelectProvider, t]);

  if (isChecking) {
    return (
      <div className="h-screen w-full bg-background-default flex items-center justify-center">
        <WelcomeAgimeLogo />
      </div>
    );
  }

  if (showOllamaSetup) {
    return <OllamaSetup onSuccess={handleOllamaComplete} onCancel={handleOllamaCancel} />;
  }

  if (!hasProvider && showFirstTimeSetup) {
    return (
      <div className="h-screen w-full bg-background-default overflow-hidden">
        <div className="h-full overflow-y-auto">
          <div className="min-h-full flex flex-col items-center justify-center p-4 py-8">
            <div className="max-w-2xl w-full mx-auto p-8">
              {/* Header section */}
              <div className="text-left mb-8 sm:mb-12">
                <div className="space-y-3 sm:space-y-4">
                  <div className="origin-bottom-left agime-icon-animation">
                    <Goose className="size-6 sm:size-8" />
                  </div>
                  <h1 className="text-2xl sm:text-4xl font-light text-left">{t('title')}</h1>
                </div>
                <p className="text-text-muted text-base sm:text-lg mt-4 sm:mt-6">
                  {t('subtitle')}
                </p>
              </div>

              {/* Quick Setup - Featured Card */}
              <div
                onClick={() => setShowQuickSetup(true)}
                className="relative w-full p-5 sm:p-6 mb-6 bg-gradient-to-r from-block-teal/10 to-block-orange/5 border-2 border-block-teal/30 rounded-xl hover:border-block-teal/60 hover:shadow-lg hover:shadow-block-teal/20 transition-all duration-300 cursor-pointer group overflow-hidden"
              >
                {/* Animated gradient background */}
                <div className="absolute inset-0 bg-gradient-to-r from-block-teal/5 via-transparent to-block-orange/5 opacity-0 group-hover:opacity-100 transition-opacity duration-500"></div>

                <div className="relative flex items-center justify-between">
                  <div className="flex items-center gap-4">
                    <div className="w-12 h-12 rounded-xl bg-gradient-to-br from-block-teal to-block-teal/70 flex items-center justify-center shadow-lg shadow-block-teal/30 group-hover:shadow-xl group-hover:shadow-block-teal/40 transition-all duration-300">
                      <Sparkles className="w-6 h-6 text-white" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2">
                        <h3 className="font-semibold text-text-standard text-base sm:text-lg">
                          {t('quickSetupWizard.title')}
                        </h3>
                        <span className="px-2 py-0.5 text-xs font-medium rounded-full bg-block-teal/20 text-block-teal">
                          {t('quickSetupWizard.recommended')}
                        </span>
                      </div>
                      <p className="text-text-muted text-sm sm:text-base mt-1">
                        {t('quickSetupWizard.description')}
                      </p>
                    </div>
                  </div>
                  <div className="text-block-teal group-hover:translate-x-1 transition-transform duration-300">
                    <svg
                      className="w-6 h-6"
                      fill="none"
                      stroke="currentColor"
                      viewBox="0 0 24 24"
                    >
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M9 5l7 7-7 7"
                      />
                    </svg>
                  </div>
                </div>
              </div>

              {/* Other providers section */}
              <div className="w-full p-4 sm:p-6 bg-transparent border border-background-hover rounded-xl">
                <h3 className="font-medium text-text-standard text-sm sm:text-base mb-3">
                  {t('otherProviders.title')}
                </h3>
                <p className="text-text-muted text-sm sm:text-base mb-4">
                  {t('otherProviders.description')}
                </p>
                <button
                  onClick={() => navigate('/welcome', { replace: true })}
                  className="text-block-teal hover:text-block-teal/80 text-sm font-medium transition-colors"
                >
                  {t('otherProviders.goToSettings')}
                </button>
              </div>

              <div className="mt-6">
                <TelemetrySettings isWelcome />
              </div>
            </div>
          </div>
        </div>

        {/* Quick Setup Modal */}
        <QuickSetupModal
          open={showQuickSetup}
          onOpenChange={setShowQuickSetup}
          onComplete={handleQuickSetupComplete}
        />
      </div>
    );
  }

  return <>{children}</>;
}
