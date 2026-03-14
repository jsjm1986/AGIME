import { useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '../components/ui/card';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { apiClient } from '../api/client';
import { buildRedirectQuery, resolveSafeRedirectPath } from '../utils/navigation';

export function RegisterPage() {
  const { t } = useTranslation();
  const [email, setEmail] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [result, setResult] = useState<{ apiKey: string } | null>(null);
  const [searchParams] = useSearchParams();
  const redirectPath = resolveSafeRedirectPath(searchParams.get('redirect'));
  const loginLink = `/login${buildRedirectQuery(redirectPath)}`;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError('');

    if (password && password !== confirmPassword) {
      setError(t('register.passwordMismatch'));
      setLoading(false);
      return;
    }
    if (password && password.length < 8) {
      setError(t('register.passwordTooShort'));
      setLoading(false);
      return;
    }

    try {
      const res = await apiClient.register(email, displayName, password || undefined);
      setResult({ apiKey: res.api_key });
    } catch (err) {
      setError(err instanceof Error ? err.message : t('auth.registerFailed'));
    } finally {
      setLoading(false);
    }
  };

  if (result) {
    return (
      <div className="min-h-screen bg-[radial-gradient(circle_at_top,hsl(var(--primary))/0.08,transparent_28%),linear-gradient(180deg,hsl(var(--background)),hsl(var(--ui-shell-gradient-end)))] px-4 py-8">
        <div className="absolute top-4 right-4">
          <LanguageSwitcher />
        </div>
        <div className="mx-auto flex min-h-[80vh] w-full max-w-5xl items-center justify-center">
        <Card className="w-full max-w-md border-[hsl(var(--ui-line-soft))/0.78] bg-[hsl(var(--card))/0.92] shadow-[0_26px_54px_hsl(var(--ui-shadow)/0.12)]">
          <CardHeader>
            <CardTitle className="text-[26px]">{t('register.success')}</CardTitle>
            <CardDescription className="leading-6">{t('register.saveApiKey')}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="rounded-[14px] border border-[hsl(var(--ui-line-soft))/0.72] bg-[hsl(var(--ui-surface-panel-muted))/0.72] p-3 font-mono text-sm break-all">
              {result.apiKey}
            </div>
            <p className="text-sm leading-6 text-[hsl(var(--muted-foreground))/0.94]">
              {t('register.apiKeyWarning')}
            </p>
          </CardContent>
          <CardFooter>
            <Link to={loginLink} className="w-full">
              <Button className="w-full">{t('register.goToLogin')}</Button>
            </Link>
          </CardFooter>
        </Card>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,hsl(var(--primary))/0.08,transparent_28%),linear-gradient(180deg,hsl(var(--background)),hsl(var(--ui-shell-gradient-end)))] px-4 py-8">
      <div className="absolute top-4 right-4">
        <LanguageSwitcher />
      </div>
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 lg:flex-row lg:items-start lg:justify-between">
      <div className="max-w-lg space-y-4 px-2 pt-10 lg:pt-20">
        <div className="inline-flex items-center gap-2 rounded-full border border-[hsl(var(--ui-line-soft))/0.74] bg-[hsl(var(--ui-surface-panel-strong))/0.78] px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-[hsl(var(--muted-foreground))/0.88]">
          {t('auth.register')}
        </div>
        <div className="space-y-3">
          <h1 className="font-display text-[34px] font-semibold tracking-[-0.04em] text-[hsl(var(--foreground))] md:text-[42px]">
            {t('auth.register')}
          </h1>
          <p className="max-w-xl text-sm leading-7 text-[hsl(var(--muted-foreground))/0.94] md:text-[15px]">
            {t('auth.registerDescription')}
          </p>
        </div>
      </div>
      <Card className="w-full max-w-md border-[hsl(var(--ui-line-soft))/0.78] bg-[hsl(var(--card))/0.92] shadow-[0_26px_54px_hsl(var(--ui-shadow)/0.12)]">
        <CardHeader>
          <CardTitle className="text-[26px]">{t('auth.register')}</CardTitle>
          <CardDescription className="leading-6">{t('auth.registerDescription')}</CardDescription>
        </CardHeader>
        <form onSubmit={handleSubmit}>
          <CardContent className="space-y-4">
            {error && (
              <div className="rounded-[14px] border border-[hsl(var(--status-error-text))/0.16] bg-[hsl(var(--status-error-bg))/0.96] p-3 text-sm text-[hsl(var(--status-error-text))]">
                {error}
              </div>
            )}
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
              <label className="text-sm font-medium">{t('register.displayName')}</label>
              <Input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder={t('register.namePlaceholder')}
                required
              />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">{t('auth.password')}</label>
              <Input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={t('register.passwordPlaceholder')}
                minLength={8}
              />
            </div>
            {password && (
              <div className="space-y-2">
                <label className="text-sm font-medium">{t('register.confirmPassword')}</label>
                <Input
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  placeholder={t('register.confirmPasswordPlaceholder')}
                  required
                />
              </div>
            )}
          </CardContent>
          <CardFooter className="flex flex-col gap-4">
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? t('auth.registering') : t('auth.register')}
            </Button>
            <p className="text-sm text-[hsl(var(--muted-foreground))/0.94]">
              {t('auth.hasAccount')}{' '}
              <Link to={loginLink} className="text-[hsl(var(--primary))] hover:underline">
                {t('auth.login')}
              </Link>
            </p>
          </CardFooter>
        </form>
      </Card>
      </div>
    </div>
  );
}
