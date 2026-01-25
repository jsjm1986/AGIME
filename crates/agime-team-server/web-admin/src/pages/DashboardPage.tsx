import { useState } from 'react';
import { Link } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Button } from '../components/ui/button';
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '../components/ui/card';
import { LanguageSwitcher } from '../components/LanguageSwitcher';
import { useAuth } from '../contexts/AuthContext';

const SERVER_URL = window.location.origin;

export function DashboardPage() {
  const { t } = useTranslation();
  const { user, logout } = useAuth();
  const [copied, setCopied] = useState(false);

  const handleLogout = async () => {
    await logout();
  };

  const handleCopyUrl = async () => {
    await navigator.clipboard.writeText(SERVER_URL);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="min-h-screen p-4 md:p-8">
      <div className="max-w-4xl mx-auto space-y-6">
        <div className="flex justify-between items-center">
          <h1 className="text-2xl font-bold">{t('dashboard.title')}</h1>
          <div className="flex items-center gap-2">
            <LanguageSwitcher />
            <Button variant="outline" onClick={handleLogout}>
              {t('auth.logout')}
            </Button>
          </div>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>{t('dashboard.profile')}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <p><span className="font-medium">{t('common.email')}:</span> {user?.email}</p>
            <p><span className="font-medium">{t('common.name')}:</span> {user?.display_name}</p>
            <p><span className="font-medium">{t('common.id')}:</span> {user?.id}</p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t('dashboard.quickActions')}</CardTitle>
          </CardHeader>
          <CardContent className="flex gap-2">
            <Link to="/api-keys">
              <Button>{t('dashboard.manageApiKeys')}</Button>
            </Link>
            <Link to="/teams">
              <Button variant="outline">{t('dashboard.manageTeams')}</Button>
            </Link>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>{t('guide.title')}</CardTitle>
            <CardDescription>{t('guide.subtitle')}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-6">
            <div className="space-y-4">
              <div>
                <h3 className="font-semibold text-lg">{t('guide.step1Title')}</h3>
                <p className="text-[hsl(var(--muted-foreground))]">{t('guide.step1Desc')}</p>
              </div>

              <div>
                <h3 className="font-semibold text-lg">{t('guide.step2Title')}</h3>
                <p className="text-[hsl(var(--muted-foreground))] mb-3">{t('guide.step2Desc')}</p>

                <div className="bg-[hsl(var(--muted))] rounded-lg p-4 mb-4">
                  <p className="text-xs text-[hsl(var(--muted-foreground))] mb-2">{t('guide.appPath')}: {t('guide.appPathDesc')}</p>
                </div>

                <div className="bg-[hsl(var(--card))] border rounded-lg p-4 space-y-4">
                  <div className="space-y-2">
                    <label className="text-sm font-medium">{t('guide.serverUrl')} *</label>
                    <div className="flex gap-2">
                      <div className="flex-1 p-2 bg-[hsl(var(--muted))] rounded font-mono text-sm">
                        {SERVER_URL}
                      </div>
                      <Button variant="outline" size="sm" onClick={handleCopyUrl}>
                        {copied ? t('guide.copied') : t('guide.copyUrl')}
                      </Button>
                    </div>
                  </div>

                  <div className="space-y-2">
                    <label className="text-sm font-medium">{t('guide.apiKeyField')} *</label>
                    <div className="p-2 bg-[hsl(var(--muted))] rounded font-mono text-sm text-[hsl(var(--muted-foreground))]">
                      agime_xxx_...
                    </div>
                    <p className="text-xs text-[hsl(var(--muted-foreground))]">{t('guide.apiKeyNote')}</p>
                  </div>

                  <div className="space-y-2">
                    <label className="text-sm font-medium">{t('guide.displayName')}</label>
                    <div className="p-2 bg-[hsl(var(--muted))] rounded text-sm text-[hsl(var(--muted-foreground))]">
                      {t('guide.displayNameNote')}
                    </div>
                  </div>

                  <div className="pt-2 border-t">
                    <p className="text-sm text-[hsl(var(--muted-foreground))]">
                      {t('guide.testConnectionNote')}
                    </p>
                  </div>
                </div>
              </div>

              <div>
                <h3 className="font-semibold text-lg">{t('guide.step3Title')}</h3>
                <p className="text-[hsl(var(--muted-foreground))] mb-2">{t('guide.step3Desc')}</p>
                <ul className="list-disc list-inside text-[hsl(var(--muted-foreground))] space-y-1">
                  <li>{t('guide.feature1')}</li>
                  <li>{t('guide.feature2')}</li>
                  <li>{t('guide.feature3')}</li>
                </ul>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
