import { useState, useEffect, useCallback } from 'react';
import { Link, useNavigate, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '../components/ui/card';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { useAuth } from '../contexts/AuthContext';
import { useBrand } from '../contexts/BrandContext';
import { BrandOverrides } from '../api/brand';
import { Copy, Check, KeyRound, Palette } from 'lucide-react';
import { buildRedirectQuery, resolveSafeRedirectPath } from '../utils/navigation';

const EMPTY_BRAND_FIELDS: BrandOverrides = {
  name: '',
  logoText: '',
  logoUrl: '',
  websiteUrl: '',
  websiteLabel: '',
};

export function LoginPage() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<'password' | 'apikey'>('password');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const { login, loginWithPassword } = useAuth();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { brand, activate, overrides, saveOverrides } = useBrand();
  const [showLicense, setShowLicense] = useState(false);
  const [licenseKey, setLicenseKey] = useState('');
  const [licenseLoading, setLicenseLoading] = useState(false);
  const [licenseError, setLicenseError] = useState('');
  const [licenseSuccess, setLicenseSuccess] = useState('');
  const [copied, setCopied] = useState(false);

  // Brand customization state
  const [showBrand, setShowBrand] = useState(false);
  const [brandFields, setBrandFields] = useState<BrandOverrides>({ ...EMPTY_BRAND_FIELDS });
  const [brandSaving, setBrandSaving] = useState(false);
  const [brandError, setBrandError] = useState('');
  const [brandSuccess, setBrandSuccess] = useState('');
  const redirectPath = resolveSafeRedirectPath(searchParams.get('redirect'));
  const registerLink = `/register${buildRedirectQuery(redirectPath)}`;
  const systemAdminLink = `/system-admin/login${buildRedirectQuery('/system-admin')}`;

  const updateBrandField = useCallback(
    (field: keyof BrandOverrides, value: string) =>
      setBrandFields((prev) => ({ ...prev, [field]: value })),
    [],
  );

  // Sync overrides into form fields when loaded
  useEffect(() => {
    if (overrides) {
      setBrandFields({
        name: overrides.name ?? '',
        logoText: overrides.logoText ?? '',
        logoUrl: overrides.logoUrl ?? '',
        websiteUrl: overrides.websiteUrl ?? '',
        websiteLabel: overrides.websiteLabel ?? '',
      });
    }
  }, [overrides]);

  const handleCopyMachineId = async () => {
    if (!brand.machineId) return;
    await navigator.clipboard.writeText(brand.machineId);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleActivate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!licenseKey.trim()) return;
    setLicenseLoading(true);
    setLicenseError('');
    setLicenseSuccess('');
    try {
      await activate(licenseKey.trim());
      setLicenseSuccess(t('auth.activateSuccess'));
      setLicenseKey('');
    } catch (err) {
      setLicenseError(err instanceof Error ? err.message : t('auth.activateFailed'));
    } finally {
      setLicenseLoading(false);
    }
  };

  const withBrandSaving = useCallback(async (fn: () => Promise<void>) => {
    setBrandSaving(true);
    setBrandError('');
    setBrandSuccess('');
    try {
      await fn();
      setBrandSuccess(t('auth.brandSaved'));
    } catch (err) {
      setBrandError(err instanceof Error ? err.message : t('auth.brandSaveFailed'));
    } finally {
      setBrandSaving(false);
    }
  }, [t]);

  const handleBrandSave = async (e: React.FormEvent) => {
    e.preventDefault();
    await withBrandSaving(async () => {
      await saveOverrides({
        name: brandFields.name || null,
        logoText: brandFields.logoText || null,
        logoUrl: brandFields.logoUrl || null,
        websiteUrl: brandFields.websiteUrl || null,
        websiteLabel: brandFields.websiteLabel || null,
      });
    });
  };

  const handleBrandReset = async () => {
    await withBrandSaving(async () => {
      await saveOverrides({});
      setBrandFields({ ...EMPTY_BRAND_FIELDS });
    });
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError('');

    try {
      if (tab === 'password') {
        await loginWithPassword(email, password);
      } else {
        await login(apiKey);
      }
      navigate(redirectPath, { replace: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : t('auth.loginFailed'));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,hsl(var(--primary))/0.08,transparent_28%),linear-gradient(180deg,hsl(var(--background)),hsl(var(--ui-shell-gradient-end)))] px-4 py-8">
      <div className="absolute top-4 right-4">
        <LanguageSwitcher />
      </div>
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 lg:flex-row lg:items-start lg:justify-between">
      <div className="max-w-lg space-y-4 px-2 pt-10 lg:pt-20">
        <div className="inline-flex items-center gap-2 rounded-full border border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-strong))/0.78] px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-[hsl(var(--muted-foreground))/0.88]">
          {brand.name}
        </div>
        <div className="space-y-3">
          <h1 className="font-display text-[34px] font-semibold tracking-[-0.04em] text-[hsl(var(--foreground))] md:text-[42px]">
            {t('auth.login')}
          </h1>
          <p className="max-w-xl text-sm leading-7 text-[hsl(var(--muted-foreground))/0.94] md:text-[15px]">
            {t('auth.loginDescription')}
          </p>
        </div>
      </div>
      <Card className="w-full max-w-md border-[hsl(var(--ui-line-soft))/0.78] bg-[hsl(var(--card))/0.92] shadow-[0_26px_54px_hsl(var(--ui-shadow)/0.12)]">
        <CardHeader>
          <CardTitle className="text-[26px]">{t('auth.login')}</CardTitle>
          <CardDescription className="leading-6">{t('auth.loginDescription')}</CardDescription>
        </CardHeader>
        {/* Tab switcher */}
        <div className="mx-6 mb-2 flex rounded-[14px] border border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-muted))/0.6] p-1">
          <button
            type="button"
            onClick={() => setTab('password')}
            className={`flex-1 rounded-[10px] px-3 py-2 text-[13px] font-medium transition-colors ${
              tab === 'password'
                ? 'bg-[hsl(var(--ui-surface-panel-strong))/0.96] text-[hsl(var(--foreground))] shadow-[0_6px_14px_hsl(var(--ui-shadow)/0.08)]'
                : 'text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]'
            }`}
          >
            {t('auth.passwordLogin')}
          </button>
          <button
            type="button"
            onClick={() => setTab('apikey')}
            className={`flex-1 rounded-[10px] px-3 py-2 text-[13px] font-medium transition-colors ${
              tab === 'apikey'
                ? 'bg-[hsl(var(--ui-surface-panel-strong))/0.96] text-[hsl(var(--foreground))] shadow-[0_6px_14px_hsl(var(--ui-shadow)/0.08)]'
                : 'text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]'
            }`}
          >
            {t('auth.apiKeyLogin')}
          </button>
        </div>
        <form onSubmit={handleSubmit}>
          <CardContent className="space-y-4">
            {error && (
              <div className="rounded-[14px] border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.96] px-3 py-2.5 text-sm text-[hsl(var(--status-error-text))]">
                {error}
              </div>
            )}
            {tab === 'password' ? (
              <>
                <div className="space-y-2">
                  <label className="text-sm font-medium">{t('common.email')}</label>
                  <Input
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    placeholder={t('register.emailPlaceholder')}
                    required
                  />
                </div>
                <div className="space-y-2">
                  <label className="text-sm font-medium">{t('auth.password')}</label>
                  <Input
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    placeholder="••••••••"
                    required
                  />
                </div>
              </>
            ) : (
              <div className="space-y-2">
                <label className="text-sm font-medium">{t('auth.apiKey')}</label>
                <Input
                  type="password"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="sk-..."
                  required
                />
              </div>
            )}
          </CardContent>
          <CardFooter className="flex flex-col gap-4">
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? t('auth.loggingIn') : t('auth.login')}
            </Button>
            <p className="text-sm text-[hsl(var(--muted-foreground))/0.94]">
              {t('auth.noAccount')}{' '}
              <Link to={registerLink} className="text-[hsl(var(--primary))] hover:underline">
                {t('auth.register')}
              </Link>
            </p>
            <Link to={systemAdminLink} className="text-xs text-[hsl(var(--muted-foreground))/0.88] hover:text-[hsl(var(--foreground))] hover:underline">
              {t('auth.systemAdminEntry')}
            </Link>
          </CardFooter>
        </form>
      </Card>

      {/* License activation toggle */}
      <div className="mt-4 w-full max-w-md">
        <button
          type="button"
          onClick={() => setShowLicense(!showLicense)}
          className="mx-auto flex items-center gap-1.5 text-xs text-[hsl(var(--muted-foreground))/0.9] transition-colors hover:text-[hsl(var(--foreground))]"
        >
          <KeyRound className="w-3 h-3" />
          {t('auth.license')}
          {brand.licensed && (
            <span className="text-status-success-text">({t('auth.licensed')})</span>
          )}
        </button>

        {showLicense && (
          <Card className="mt-2 border-[hsl(var(--ui-line-soft))/0.78] bg-[hsl(var(--card))/0.92] shadow-[0_20px_40px_hsl(var(--ui-shadow)/0.1)]">
            <CardContent className="pt-4 space-y-3">
              {/* License status */}
              {brand.licensed && brand.licensee && (
                <div className="rounded-[12px] border border-[hsl(var(--status-success-text))/0.16] bg-[hsl(var(--status-success-bg))/0.96] p-2.5 text-xs text-[hsl(var(--status-success-text))]">
                  {t('auth.licensee')}: {brand.licensee}
                </div>
              )}

              {/* Machine ID */}
              {brand.machineId && (
                <div className="space-y-1.5">
                  <label className="text-xs font-medium text-[hsl(var(--muted-foreground))]">
                    {t('auth.machineId')}
                  </label>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 break-all rounded-[12px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-muted))/0.7] px-2.5 py-1.5 font-mono text-xs select-all">
                      {brand.machineId}
                    </code>
                    <button
                      type="button"
                      onClick={handleCopyMachineId}
                      className="shrink-0 rounded-[10px] p-1.5 transition-colors hover:bg-[hsl(var(--ui-surface-panel-muted))/0.8]"
                      title="Copy"
                    >
                      {copied ? <Check className="h-3.5 w-3.5 text-[hsl(var(--status-success-text))]" /> : <Copy className="h-3.5 w-3.5" />}
                    </button>
                  </div>
                  <p className="text-caption text-[hsl(var(--muted-foreground))/0.9]">
                    {t('auth.machineIdHint')}
                  </p>
                </div>
              )}

              {/* License key input — always visible for activate or update */}
              <form onSubmit={handleActivate} className="space-y-2">
                {licenseError && (
                  <div className="rounded-[12px] border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.96] p-2 text-xs text-[hsl(var(--status-error-text))]">
                    {licenseError}
                  </div>
                )}
                {licenseSuccess && (
                  <div className="rounded-[12px] border border-[hsl(var(--status-success-text))/0.16] bg-[hsl(var(--status-success-bg))/0.96] p-2 text-xs text-[hsl(var(--status-success-text))]">
                    {licenseSuccess}
                  </div>
                )}
                {brand.licensed && (
                  <p className="text-caption text-[hsl(var(--muted-foreground))/0.9]">
                    {t('auth.updateLicenseHint')}
                  </p>
                )}
                <Input
                  value={licenseKey}
                  onChange={(e) => setLicenseKey(e.target.value)}
                  placeholder={t('auth.licenseKeyPlaceholder')}
                  className="text-xs font-mono"
                />
                <Button
                  type="submit"
                  variant="outline"
                  size="sm"
                  className="w-full"
                  disabled={licenseLoading || !licenseKey.trim()}
                >
                  {licenseLoading && t('auth.activating')}
                  {!licenseLoading && brand.licensed && t('auth.updateLicense')}
                  {!licenseLoading && !brand.licensed && t('auth.activate')}
                </Button>
              </form>

              {/* Brand customization — only when licensed */}
              {brand.licensed && (
                <>
                  <div className="my-2 border-t border-[hsl(var(--ui-line-soft))/0.72]" />
                  <button
                    type="button"
                    onClick={() => setShowBrand(!showBrand)}
                    className="flex items-center gap-1.5 text-xs text-[hsl(var(--muted-foreground))/0.9] transition-colors hover:text-[hsl(var(--foreground))]"
                  >
                    <Palette className="w-3 h-3" />
                    {t('auth.brandCustomize')}
                  </button>

                  {showBrand && (
                    <form onSubmit={handleBrandSave} className="space-y-2">
                      {brandError && (
                        <div className="rounded-[12px] border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.96] p-2 text-xs text-[hsl(var(--status-error-text))]">
                          {brandError}
                        </div>
                      )}
                      {brandSuccess && (
                        <div className="rounded-[12px] border border-[hsl(var(--status-success-text))/0.16] bg-[hsl(var(--status-success-bg))/0.96] p-2 text-xs text-[hsl(var(--status-success-text))]">
                          {brandSuccess}
                        </div>
                      )}
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))/0.9]">{t('auth.brandName')}</label>
                        <Input value={brandFields.name ?? ''} onChange={(e) => updateBrandField('name', e.target.value)} placeholder={t('auth.brandNamePlaceholder')} className="text-xs" />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))/0.9]">{t('auth.brandLogoText')}</label>
                        <Input value={brandFields.logoText ?? ''} onChange={(e) => updateBrandField('logoText', e.target.value)} placeholder={t('auth.brandLogoTextPlaceholder')} className="text-xs" maxLength={2} />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))/0.9]">{t('auth.brandLogoUrl')}</label>
                        <Input value={brandFields.logoUrl ?? ''} onChange={(e) => updateBrandField('logoUrl', e.target.value)} placeholder={t('auth.brandLogoUrlPlaceholder')} className="text-xs" />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))/0.9]">{t('auth.brandWebsiteUrl')}</label>
                        <Input value={brandFields.websiteUrl ?? ''} onChange={(e) => updateBrandField('websiteUrl', e.target.value)} placeholder={t('auth.brandWebsiteUrlPlaceholder')} className="text-xs" />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))/0.9]">{t('auth.brandWebsiteLabel')}</label>
                        <Input value={brandFields.websiteLabel ?? ''} onChange={(e) => updateBrandField('websiteLabel', e.target.value)} placeholder={t('auth.brandWebsiteLabelPlaceholder')} className="text-xs" />
                      </div>
                      <div className="flex gap-2">
                        <Button type="submit" variant="outline" size="sm" className="flex-1" disabled={brandSaving}>
                          {brandSaving ? t('auth.brandSaving') : t('auth.brandSave')}
                        </Button>
                        <Button type="button" variant="ghost" size="sm" onClick={handleBrandReset} disabled={brandSaving}>
                          {t('auth.brandReset')}
                        </Button>
                      </div>
                    </form>
                  )}
                </>
              )}
            </CardContent>
          </Card>
        )}
      </div>
      </div>
    </div>
  );
}
