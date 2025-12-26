import { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../../ui/button';
import { Settings, RefreshCw, ExternalLink, Monitor, Palette, Globe, HelpCircle, Info, Download } from 'lucide-react';
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '../../ui/dialog';
import UpdateSection from './UpdateSection';
import TunnelSection from '../tunnel/TunnelSection';

import { COST_TRACKING_ENABLED, UPDATES_ENABLED } from '../../../updates';
import { getApiUrl } from '../../../config';
import ThemeSelector from '../../GooseSidebar/ThemeSelector';
import BlockLogoBlack from './icons/block-lockup_black.png';
import BlockLogoWhite from './icons/block-lockup_white.png';
import TelemetrySettings from './TelemetrySettings';
import LanguageSelector from '../LanguageSelector';
import { getConfigCompat } from '../../../utils/envCompat';
import { SettingsCard, SettingsToggleItem, SettingsItem } from '../common';

interface AppSettingsSectionProps {
  scrollToSection?: string;
}

export default function AppSettingsSection({ scrollToSection }: AppSettingsSectionProps) {
  const { t } = useTranslation('settings');
  const { t: tCommon } = useTranslation('common');
  const [menuBarIconEnabled, setMenuBarIconEnabled] = useState(true);
  const [dockIconEnabled, setDockIconEnabled] = useState(true);
  const [wakelockEnabled, setWakelockEnabled] = useState(true);
  const [isMacOS, setIsMacOS] = useState(false);
  const [isDockSwitchDisabled, setIsDockSwitchDisabled] = useState(false);
  const [showNotificationModal, setShowNotificationModal] = useState(false);
  const [pricingStatus, setPricingStatus] = useState<'loading' | 'success' | 'error'>('loading');
  const [lastFetchTime, setLastFetchTime] = useState<Date | null>(null);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [showPricing, setShowPricing] = useState(true);
  const [isDarkMode, setIsDarkMode] = useState(false);
  const updateSectionRef = useRef<HTMLDivElement>(null);

  // Check if GOOSE_VERSION is set to determine if Updates section should be shown
  const shouldShowUpdates = !getConfigCompat('VERSION');

  // Check if running on macOS
  useEffect(() => {
    setIsMacOS(window.electron.platform === 'darwin');
  }, []);

  // Detect theme changes
  useEffect(() => {
    const updateTheme = () => {
      setIsDarkMode(document.documentElement.classList.contains('dark'));
    };

    // Initial check
    updateTheme();

    // Listen for theme changes
    const observer = new MutationObserver(updateTheme);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    });

    return () => observer.disconnect();
  }, []);

  // Load show pricing setting
  useEffect(() => {
    const stored = localStorage.getItem('show_pricing');
    setShowPricing(stored !== 'false');
  }, []);

  // Check pricing status on mount
  useEffect(() => {
    checkPricingStatus();
  }, []);

  const checkPricingStatus = async () => {
    try {
      const apiUrl = getApiUrl('/config/pricing');
      const secretKey = await window.electron.getSecretKey();

      const headers: HeadersInit = { 'Content-Type': 'application/json' };
      if (secretKey) {
        headers['X-Secret-Key'] = secretKey;
      }

      const response = await fetch(apiUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify({ configured_only: true }),
      });

      if (response.ok) {
        await response.json();
        setPricingStatus('success');
        setLastFetchTime(new Date());
      } else {
        setPricingStatus('error');
      }
    } catch {
      setPricingStatus('error');
    }
  };

  const handleRefreshPricing = async () => {
    setIsRefreshing(true);
    try {
      const apiUrl = getApiUrl('/config/pricing');
      const secretKey = await window.electron.getSecretKey();

      const headers: HeadersInit = { 'Content-Type': 'application/json' };
      if (secretKey) {
        headers['X-Secret-Key'] = secretKey;
      }

      const response = await fetch(apiUrl, {
        method: 'POST',
        headers,
        body: JSON.stringify({ configured_only: false }),
      });

      if (response.ok) {
        setPricingStatus('success');
        setLastFetchTime(new Date());
        // Trigger a reload of the cost database
        window.dispatchEvent(new CustomEvent('pricing-updated'));
      } else {
        setPricingStatus('error');
      }
    } catch {
      setPricingStatus('error');
    } finally {
      setIsRefreshing(false);
    }
  };

  // Handle scrolling to update section
  useEffect(() => {
    if (scrollToSection === 'update' && updateSectionRef.current) {
      // Use a timeout to ensure the DOM is ready
      setTimeout(() => {
        updateSectionRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
      }, 100);
    }
  }, [scrollToSection]);

  // Load menu bar and dock icon states
  useEffect(() => {
    window.electron.getMenuBarIconState().then((enabled) => {
      setMenuBarIconEnabled(enabled);
    });

    window.electron.getWakelockState().then((enabled) => {
      setWakelockEnabled(enabled);
    });

    if (isMacOS) {
      window.electron.getDockIconState().then((enabled) => {
        setDockIconEnabled(enabled);
      });
    }
  }, [isMacOS]);

  const handleMenuBarIconToggle = async (newState: boolean) => {
    // If we're turning off the menu bar icon and the dock icon is hidden,
    // we need to show the dock icon to maintain accessibility
    if (!newState && !dockIconEnabled && isMacOS) {
      const success = await window.electron.setDockIcon(true);
      if (success) {
        setDockIconEnabled(true);
      }
    }
    const success = await window.electron.setMenuBarIcon(newState);
    if (success) {
      setMenuBarIconEnabled(newState);
    }
  };

  const handleDockIconToggle = async (newState: boolean) => {
    // If we're turning off the dock icon and the menu bar icon is hidden,
    // we need to show the menu bar icon to maintain accessibility
    if (!newState && !menuBarIconEnabled) {
      const success = await window.electron.setMenuBarIcon(true);
      if (success) {
        setMenuBarIconEnabled(true);
      }
    }

    // Disable the switch to prevent rapid toggling
    setIsDockSwitchDisabled(true);
    setTimeout(() => {
      setIsDockSwitchDisabled(false);
    }, 1000);

    // Set the dock icon state
    const success = await window.electron.setDockIcon(newState);
    if (success) {
      setDockIconEnabled(newState);
    }
  };

  const handleWakelockToggle = async (newState: boolean) => {
    const success = await window.electron.setWakelock(newState);
    if (success) {
      setWakelockEnabled(newState);
    }
  };

  const handleShowPricingToggle = (checked: boolean) => {
    setShowPricing(checked);
    localStorage.setItem('show_pricing', String(checked));
    // Trigger storage event for other components
    window.dispatchEvent(new CustomEvent('storage'));
  };

  return (
    <div className="space-y-6 pb-8 mt-1">
      {/* 外观设置 */}
      <SettingsCard
        icon={<Monitor className="h-5 w-5" />}
        title={t('app.appearance')}
        description={t('app.appearanceDescription')}
      >
        {/* 通知 */}
        <SettingsItem
          title={t('app.notifications')}
          description={
            <>
              {t('app.notificationsManaged')}{' - '}
              <span
                className="underline hover:cursor-pointer text-text-muted hover:text-text-default"
                onClick={() => setShowNotificationModal(true)}
              >
                {t('app.configurationGuide')}
              </span>
            </>
          }
          control={
            <Button
              className="flex items-center gap-2 justify-center"
              variant="secondary"
              size="sm"
              onClick={async () => {
                try {
                  await window.electron.openNotificationsSettings();
                } catch (error) {
                  console.error('Failed to open notification settings:', error);
                }
              }}
            >
              <Settings className="w-4 h-4" />
              {t('app.openSettings')}
            </Button>
          }
        />

        {/* 菜单栏图标 */}
        <SettingsToggleItem
          title={t('app.menuBarIcon')}
          description={t('app.menuBarIconDescription')}
          checked={menuBarIconEnabled}
          onCheckedChange={handleMenuBarIconToggle}
        />

        {/* 程序坞图标 (仅 macOS) */}
        {isMacOS && (
          <SettingsToggleItem
            title={t('app.dockIcon')}
            description={t('app.dockIconDescription')}
            checked={dockIconEnabled}
            onCheckedChange={handleDockIconToggle}
            disabled={isDockSwitchDisabled}
          />
        )}

        {/* 防止休眠 */}
        <SettingsToggleItem
          title={t('app.preventSleep')}
          description={t('app.preventSleepDescription')}
          checked={wakelockEnabled}
          onCheckedChange={handleWakelockToggle}
        />

        {/* 费用追踪 */}
        {COST_TRACKING_ENABLED && (
          <SettingsToggleItem
            title={t('app.costTracking')}
            description={t('app.costTrackingDescription')}
            checked={showPricing}
            onCheckedChange={handleShowPricingToggle}
          >
            {/* 费用追踪详情 */}
            <div className="space-y-2 text-xs">
              <div className="flex items-center justify-between">
                <span className="text-text-muted">{t('app.pricingSource')}:</span>
                <a
                  href="https://openrouter.ai/docs#models"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 dark:text-blue-400 hover:underline flex items-center gap-1"
                >
                  OpenRouter Docs
                  <ExternalLink size={10} />
                </a>
              </div>

              <div className="flex items-center justify-between">
                <span className="text-text-muted">{t('app.status')}:</span>
                <div className="flex items-center gap-2">
                  <span
                    className={`font-medium ${
                      pricingStatus === 'success'
                        ? 'text-green-600 dark:text-green-400'
                        : pricingStatus === 'error'
                          ? 'text-red-600 dark:text-red-400'
                          : 'text-text-muted'
                    }`}
                  >
                    {pricingStatus === 'success'
                      ? `✓ ${t('app.connected')}`
                      : pricingStatus === 'error'
                        ? `✗ ${t('app.failed')}`
                        : `... ${t('app.checking')}`}
                  </span>
                  <button
                    className="p-0.5 hover:bg-background-muted rounded transition-colors disabled:opacity-50"
                    onClick={handleRefreshPricing}
                    disabled={isRefreshing}
                    title={tCommon('refresh')}
                    type="button"
                  >
                    <RefreshCw
                      size={12}
                      className={`text-text-muted hover:text-text-default ${isRefreshing ? 'animate-spin-fast' : ''}`}
                    />
                  </button>
                </div>
              </div>

              {lastFetchTime && (
                <div className="flex items-center justify-between">
                  <span className="text-text-muted">{t('app.lastUpdated')}:</span>
                  <span className="text-text-muted">{lastFetchTime.toLocaleTimeString()}</span>
                </div>
              )}

              {pricingStatus === 'error' && (
                <p className="text-xs text-red-600 dark:text-red-400">
                  {t('app.unableToFetchPricing')}
                </p>
              )}
            </div>
          </SettingsToggleItem>
        )}
      </SettingsCard>

      {/* 主题设置 */}
      <SettingsCard
        icon={<Palette className="h-5 w-5" />}
        title={t('app.theme')}
        description={t('app.themeDescription')}
      >
        <ThemeSelector className="w-auto" hideTitle horizontal />
      </SettingsCard>

      {/* 语言设置 */}
      <SettingsCard
        icon={<Globe className="h-5 w-5" />}
        title={t('app.language')}
        description={t('app.languageDescription')}
      >
        <LanguageSelector />
      </SettingsCard>

      {/* 远程访问 */}
      <TunnelSection />

      {/* 隐私设置 */}
      <TelemetrySettings isWelcome={false} />

      {/* 帮助与反馈 */}
      <SettingsCard
        icon={<HelpCircle className="h-5 w-5" />}
        title={t('app.helpFeedback')}
        description={t('app.helpFeedbackDescription')}
      >
        <div className="flex space-x-4">
          <Button
            onClick={() => {
              window.open(
                'https://github.com/jsjm1986/AGIME/issues/new?template=bug_report.md',
                '_blank'
              );
            }}
            variant="secondary"
            size="sm"
          >
            {t('app.reportBug')}
          </Button>
          <Button
            onClick={() => {
              window.open(
                'https://github.com/jsjm1986/AGIME/issues/new?template=feature_request.md',
                '_blank'
              );
            }}
            variant="secondary"
            size="sm"
          >
            {t('app.requestFeature')}
          </Button>
        </div>
      </SettingsCard>

      {/* 版本信息 - 仅当 GOOSE_VERSION 已设置时显示 */}
      {!shouldShowUpdates && (
        <SettingsCard
          icon={<Info className="h-5 w-5" />}
          title={t('app.version')}
        >
          <div className="flex items-center gap-3">
            <img
              src={isDarkMode ? BlockLogoWhite : BlockLogoBlack}
              alt="Block Logo"
              className="h-8 w-auto"
            />
            <span className="text-2xl font-mono text-text-default">
              {String(getConfigCompat('VERSION') || t('app.development'))}
            </span>
          </div>
        </SettingsCard>
      )}

      {/* 更新设置 - 仅当 GOOSE_VERSION 未设置时显示 */}
      {UPDATES_ENABLED && shouldShowUpdates && (
        <div ref={updateSectionRef}>
          <SettingsCard
            icon={<Download className="h-5 w-5" />}
            title={t('app.updates')}
            description={t('app.updatesDescription')}
          >
            <UpdateSection />
          </SettingsCard>
        </div>
      )}

      {/* 通知说明弹窗 */}
      <Dialog
        open={showNotificationModal}
        onOpenChange={(open) => !open && setShowNotificationModal(false)}
      >
        <DialogContent className="sm:max-w-[500px]">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Settings className="text-text-muted" size={24} />
              {t('app.howToEnableNotifications')}
            </DialogTitle>
          </DialogHeader>

          <div className="py-4">
            {/* OS-specific instructions */}
            {isMacOS ? (
              <div className="space-y-4">
                <p className="text-sm text-text-default">{t('app.notificationsMacOS.intro')}</p>
                <ol className="list-decimal pl-5 space-y-2 text-sm text-text-muted">
                  <li>{t('app.notificationsMacOS.step1')}</li>
                  <li>{t('app.notificationsMacOS.step2')}</li>
                  <li>{t('app.notificationsMacOS.step3')}</li>
                  <li>{t('app.notificationsMacOS.step4')}</li>
                </ol>
              </div>
            ) : (
              <div className="space-y-4">
                <p className="text-sm text-text-default">{t('app.notificationsWindows.intro')}</p>
                <ol className="list-decimal pl-5 space-y-2 text-sm text-text-muted">
                  <li>{t('app.notificationsWindows.step1')}</li>
                  <li>{t('app.notificationsWindows.step2')}</li>
                  <li>{t('app.notificationsWindows.step3')}</li>
                  <li>{t('app.notificationsWindows.step4')}</li>
                </ol>
              </div>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setShowNotificationModal(false)}>
              {tCommon('close')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
