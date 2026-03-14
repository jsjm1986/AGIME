import { useMemo, useState } from 'react';
import { Link, Navigate, useNavigate, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ArrowRight, ShieldCheck, Sparkles, UserCog } from 'lucide-react';
import { Button } from '../components/ui/button';
import { Input } from '../components/ui/input';
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '../components/ui/card';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { useAuth } from '../contexts/AuthContext';
import { useBrand } from '../contexts/BrandContext';
import { buildRedirectQuery, resolveSafeRedirectPath } from '../utils/navigation';

export function SystemAdminLoginPage() {
  const { t } = useTranslation();
  const { brand } = useBrand();
  const { user, isAdmin, loginSystemAdmin, logout } = useAuth();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();

  const [username, setUsername] = useState('agime');
  const [password, setPassword] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const redirectPath = resolveSafeRedirectPath(searchParams.get('redirect'), '/system-admin');
  const normalLoginLink = useMemo(
    () => `/login${buildRedirectQuery(redirectPath)}`,
    [redirectPath]
  );

  if (user && isAdmin) {
    return <Navigate to={redirectPath} replace />;
  }

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setLoading(true);
    setError('');

    try {
      await loginSystemAdmin(username.trim(), password);
      navigate(redirectPath, { replace: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : t('auth.loginFailed'));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="relative min-h-screen overflow-hidden bg-[linear-gradient(135deg,#f4ede4_0%,#efe5d9_34%,#eadfce_100%)] px-4 py-8 text-[hsl(224_30%_16%)]">
      <div className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="absolute left-[-12%] top-[-10%] h-[28rem] w-[28rem] rounded-full bg-[radial-gradient(circle,rgba(201,119,52,0.22)_0%,rgba(201,119,52,0)_68%)]" />
        <div className="absolute bottom-[-16%] right-[-8%] h-[30rem] w-[30rem] rounded-full bg-[radial-gradient(circle,rgba(39,74,122,0.18)_0%,rgba(39,74,122,0)_70%)]" />
        <div className="absolute inset-x-0 top-[18%] h-px bg-[linear-gradient(90deg,transparent,rgba(80,55,28,0.18),transparent)]" />
      </div>

      <div className="absolute right-4 top-4">
        <LanguageSwitcher className="rounded-full border border-[rgba(80,55,28,0.12)] bg-[rgba(255,250,244,0.86)] px-4 text-[hsl(224_20%_22%)] shadow-[0_12px_30px_rgba(64,42,16,0.08)] hover:bg-white" />
      </div>

      <div className="relative mx-auto flex min-h-[calc(100vh-4rem)] w-full max-w-6xl items-center justify-between gap-10 lg:flex-row">
        <div className="max-w-2xl space-y-8">
          <div className="inline-flex items-center gap-2 rounded-full border border-[rgba(80,55,28,0.14)] bg-[rgba(255,250,244,0.72)] px-3 py-1.5 text-[11px] font-semibold uppercase tracking-[0.12em] text-[hsl(28_45%_28%)] shadow-[0_10px_28px_rgba(64,42,16,0.05)]">
            <ShieldCheck className="h-3.5 w-3.5" />
            {t('systemAdminLogin.badge')}
          </div>
          <div className="space-y-5">
            <h1 className="font-display max-w-xl text-[42px] font-semibold tracking-[-0.05em] text-[hsl(224_26%_14%)] md:text-[58px]">
              {t('systemAdminLogin.title')}
            </h1>
            <p className="max-w-2xl text-[16px] leading-8 text-[hsl(224_14%_30%)]">
              {t('systemAdminLogin.description')}
            </p>
          </div>
          <div className="grid gap-3 md:grid-cols-3">
            <div className="rounded-[24px] border border-[rgba(80,55,28,0.12)] bg-[rgba(255,251,247,0.74)] p-5 shadow-[0_18px_38px_rgba(64,42,16,0.08)] backdrop-blur-xl">
              <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-[hsl(28_30%_42%)]">
                {t('systemAdminLogin.firstRun')}
              </p>
              <p className="mt-3 text-sm leading-6 text-[hsl(224_14%_28%)]">
                {t('systemAdminLogin.firstRunHint')}
              </p>
            </div>
            <div className="rounded-[24px] border border-[rgba(80,55,28,0.12)] bg-[rgba(255,251,247,0.74)] p-5 shadow-[0_18px_38px_rgba(64,42,16,0.08)] backdrop-blur-xl">
              <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-[hsl(28_30%_42%)]">
                {t('systemAdminLogin.routeLabel')}
              </p>
              <p className="mt-3 font-mono text-sm text-[hsl(224_24%_22%)]">/system-admin/login</p>
            </div>
            <div className="rounded-[24px] border border-[rgba(80,55,28,0.12)] bg-[rgba(255,251,247,0.74)] p-5 shadow-[0_18px_38px_rgba(64,42,16,0.08)] backdrop-blur-xl">
              <p className="text-[11px] font-semibold uppercase tracking-[0.12em] text-[hsl(28_30%_42%)]">
                {t('systemAdminLogin.requirementLabel')}
              </p>
              <p className="mt-3 text-sm leading-6 text-[hsl(224_14%_28%)]">
                {t('systemAdminLogin.requirementHint')}
              </p>
            </div>
          </div>
        </div>

        <Card className="w-full max-w-md border-[rgba(80,55,28,0.12)] bg-[rgba(255,251,247,0.9)] text-[hsl(224_30%_16%)] shadow-[0_28px_80px_rgba(64,42,16,0.18)] backdrop-blur-2xl">
          <CardHeader>
            <div className="mb-3 inline-flex h-11 w-11 items-center justify-center rounded-[16px] border border-[rgba(80,55,28,0.12)] bg-[rgba(201,119,52,0.1)] text-[hsl(28_62%_34%)]">
              <UserCog className="h-5 w-5" />
            </div>
            <CardTitle className="text-[28px] text-[hsl(224_26%_14%)]">{t('systemAdminLogin.formTitle')}</CardTitle>
            <CardDescription className="leading-6 text-[hsl(224_14%_34%)]">
              {t('systemAdminLogin.formDescription', { brand: brand.name })}
            </CardDescription>
          </CardHeader>

          <form onSubmit={handleSubmit}>
            <CardContent className="space-y-4">
              {user && !isAdmin && (
                <div className="rounded-[18px] border border-[rgba(179,104,32,0.24)] bg-[rgba(201,119,52,0.1)] px-4 py-3 text-sm text-[hsl(28_68%_28%)]">
                  <p>{t('systemAdminLogin.activeSession')}</p>
                  <Button type="button" variant="ghost" size="sm" className="mt-2 h-7 px-2 text-[hsl(28_68%_28%)] hover:bg-[rgba(201,119,52,0.08)]" onClick={() => logout()}>
                    {t('systemAdminLogin.switchSession')}
                  </Button>
                </div>
              )}
              {error && (
                <div className="rounded-[18px] border border-[rgba(173,57,57,0.18)] bg-[rgba(173,57,57,0.08)] px-4 py-3 text-sm text-[hsl(0_54%_38%)]">
                  {error}
                </div>
              )}
              <div className="space-y-2">
                <label className="text-sm font-medium text-[hsl(224_20%_22%)]">{t('systemAdminLogin.username')}</label>
                <Input
                  value={username}
                  onChange={(event) => setUsername(event.target.value)}
                  placeholder="agime"
                  autoComplete="username"
                  className="h-12 border-[rgba(80,55,28,0.12)] bg-[rgba(246,239,229,0.88)] text-[hsl(224_26%_16%)] placeholder:text-[hsl(224_10%_54%)]"
                  required
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium text-[hsl(224_20%_22%)]">{t('auth.password')}</label>
                <Input
                  type="password"
                  value={password}
                  onChange={(event) => setPassword(event.target.value)}
                  placeholder="agime"
                  autoComplete="current-password"
                  className="h-12 border-[rgba(80,55,28,0.12)] bg-[rgba(246,239,229,0.88)] text-[hsl(224_26%_16%)] placeholder:text-[hsl(224_10%_54%)]"
                  required
                />
              </div>
              <div className="rounded-[20px] border border-[rgba(80,55,28,0.12)] bg-[linear-gradient(135deg,rgba(33,53,83,0.94),rgba(23,38,63,0.96))] px-4 py-4 text-sm text-[rgba(245,239,228,0.88)] shadow-[0_18px_36px_rgba(23,38,63,0.18)]">
                <div className="flex items-start gap-3">
                  <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-[rgba(245,211,160,0.92)]" />
                  <div className="space-y-1.5">
                    <p className="font-medium text-white">{t('systemAdminLogin.securityWarning')}</p>
                    <p className="text-xs leading-5 text-[rgba(245,239,228,0.76)]">
                      {t('systemAdminLogin.passwordResetHint')}
                    </p>
                  </div>
                </div>
              </div>
            </CardContent>
            <CardFooter className="flex flex-col gap-4">
              <Button type="submit" className="h-12 w-full gap-2 bg-[hsl(222_64%_47%)] text-white hover:bg-[hsl(222_64%_42%)]" disabled={loading}>
                <span>{loading ? t('auth.loggingIn') : t('systemAdminLogin.submit')}</span>
                {!loading && <ArrowRight className="h-4 w-4" />}
              </Button>
              <div className="flex w-full items-center justify-between text-sm text-[hsl(224_12%_40%)]">
                <Link to={normalLoginLink} className="font-medium text-[hsl(224_18%_28%)] hover:text-[hsl(224_22%_18%)] hover:underline">
                  {t('systemAdminLogin.normalLogin')}
                </Link>
                <span className="font-mono text-[11px] uppercase tracking-[0.08em] text-[hsl(28_26%_42%)]">
                  {brand.name}
                </span>
              </div>
            </CardFooter>
          </form>
        </Card>
      </div>
    </div>
  );
}
