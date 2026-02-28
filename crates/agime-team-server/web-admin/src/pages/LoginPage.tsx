import { useState, useEffect, useCallback } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '../components/ui/card';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { useAuth } from '../contexts/AuthContext';
import { useBrand } from '../contexts/BrandContext';
import { BrandOverrides } from '../api/brand';
import { Copy, Check, KeyRound, Palette } from 'lucide-react';

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
      navigate('/dashboard');
    } catch (err) {
      setError(err instanceof Error ? err.message : t('auth.loginFailed'));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex flex-col items-center justify-center p-4">
      <div className="absolute top-4 right-4">
        <LanguageSwitcher />
      </div>
      <Card className="w-full max-w-md">
        <CardHeader>
          <CardTitle>{t('auth.login')}</CardTitle>
          <CardDescription>{t('auth.loginDescription')}</CardDescription>
        </CardHeader>
        {/* Tab switcher */}
        <div className="flex border-b border-[hsl(var(--border))] mx-6 mb-2">
          <button
            type="button"
            onClick={() => setTab('password')}
            className={`flex-1 pb-2 text-sm font-medium border-b-2 transition-colors ${
              tab === 'password'
                ? 'border-[hsl(var(--primary))] text-[hsl(var(--primary))]'
                : 'border-transparent text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]'
            }`}
          >
            {t('auth.passwordLogin')}
          </button>
          <button
            type="button"
            onClick={() => setTab('apikey')}
            className={`flex-1 pb-2 text-sm font-medium border-b-2 transition-colors ${
              tab === 'apikey'
                ? 'border-[hsl(var(--primary))] text-[hsl(var(--primary))]'
                : 'border-transparent text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))]'
            }`}
          >
            {t('auth.apiKeyLogin')}
          </button>
        </div>
        <form onSubmit={handleSubmit}>
          <CardContent className="space-y-4">
            {error && (
              <div className="p-3 text-sm text-[hsl(var(--destructive))] bg-[hsl(var(--destructive))]/10 rounded-md">
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
            <p className="text-sm text-[hsl(var(--muted-foreground))]">
              {t('auth.noAccount')}{' '}
              <Link to="/register" className="text-[hsl(var(--primary))] hover:underline">
                {t('auth.register')}
              </Link>
            </p>
          </CardFooter>
        </form>
      </Card>

      {/* License activation toggle */}
      <div className="mt-4 w-full max-w-md">
        <button
          type="button"
          onClick={() => setShowLicense(!showLicense)}
          className="flex items-center gap-1.5 text-xs text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))] transition-colors mx-auto"
        >
          <KeyRound className="w-3 h-3" />
          {t('auth.license')}
          {brand.licensed && (
            <span className="text-green-500">({t('auth.licensed')})</span>
          )}
        </button>

        {showLicense && (
          <Card className="mt-2">
            <CardContent className="pt-4 space-y-3">
              {/* License status */}
              {brand.licensed && brand.licensee && (
                <div className="p-2.5 text-xs bg-green-500/10 text-green-600 dark:text-green-400 rounded-md">
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
                    <code className="flex-1 text-xs bg-[hsl(var(--muted))] px-2.5 py-1.5 rounded font-mono select-all break-all">
                      {brand.machineId}
                    </code>
                    <button
                      type="button"
                      onClick={handleCopyMachineId}
                      className="shrink-0 p-1.5 rounded hover:bg-[hsl(var(--muted))] transition-colors"
                      title="Copy"
                    >
                      {copied ? <Check className="w-3.5 h-3.5 text-green-500" /> : <Copy className="w-3.5 h-3.5" />}
                    </button>
                  </div>
                  <p className="text-caption text-[hsl(var(--muted-foreground))]">
                    {t('auth.machineIdHint')}
                  </p>
                </div>
              )}

              {/* License key input — always visible for activate or update */}
              <form onSubmit={handleActivate} className="space-y-2">
                {licenseError && (
                  <div className="p-2 text-xs text-[hsl(var(--destructive))] bg-[hsl(var(--destructive))]/10 rounded-md">
                    {licenseError}
                  </div>
                )}
                {licenseSuccess && (
                  <div className="p-2 text-xs text-green-600 dark:text-green-400 bg-green-500/10 rounded-md">
                    {licenseSuccess}
                  </div>
                )}
                {brand.licensed && (
                  <p className="text-caption text-[hsl(var(--muted-foreground))]">
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
                  <div className="border-t border-[hsl(var(--border))] my-2" />
                  <button
                    type="button"
                    onClick={() => setShowBrand(!showBrand)}
                    className="flex items-center gap-1.5 text-xs text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--foreground))] transition-colors"
                  >
                    <Palette className="w-3 h-3" />
                    {t('auth.brandCustomize')}
                  </button>

                  {showBrand && (
                    <form onSubmit={handleBrandSave} className="space-y-2">
                      {brandError && (
                        <div className="p-2 text-xs text-[hsl(var(--destructive))] bg-[hsl(var(--destructive))]/10 rounded-md">
                          {brandError}
                        </div>
                      )}
                      {brandSuccess && (
                        <div className="p-2 text-xs text-green-600 dark:text-green-400 bg-green-500/10 rounded-md">
                          {brandSuccess}
                        </div>
                      )}
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))]">{t('auth.brandName')}</label>
                        <Input value={brandFields.name ?? ''} onChange={(e) => updateBrandField('name', e.target.value)} placeholder={t('auth.brandNamePlaceholder')} className="text-xs" />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))]">{t('auth.brandLogoText')}</label>
                        <Input value={brandFields.logoText ?? ''} onChange={(e) => updateBrandField('logoText', e.target.value)} placeholder={t('auth.brandLogoTextPlaceholder')} className="text-xs" maxLength={2} />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))]">{t('auth.brandLogoUrl')}</label>
                        <Input value={brandFields.logoUrl ?? ''} onChange={(e) => updateBrandField('logoUrl', e.target.value)} placeholder={t('auth.brandLogoUrlPlaceholder')} className="text-xs" />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))]">{t('auth.brandWebsiteUrl')}</label>
                        <Input value={brandFields.websiteUrl ?? ''} onChange={(e) => updateBrandField('websiteUrl', e.target.value)} placeholder={t('auth.brandWebsiteUrlPlaceholder')} className="text-xs" />
                      </div>
                      <div className="space-y-1.5">
                        <label className="text-xs font-medium text-[hsl(var(--muted-foreground))]">{t('auth.brandWebsiteLabel')}</label>
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
  );
}
